//! Linux evdev device source. The device is opt-in and opened only in displayd.
//! Raw events never enter the display renderer. They are translated into a
//! bounded typed event and sent directly to the waylandd service.
use anyhow::{Context, Result, bail};
use std::{
    fs::{File, OpenOptions},
    io,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};
use waybroker_common::{
    IpcEnvelope, MessageKind, PhysicalInputEvent, PhysicalInputKind, ServiceRole, WaylandCommand,
    WaylandEvent, connect_service_socket, read_ipc_envelope, send_ipc_envelope,
};

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const FIRST_POINTER_BUTTON: u16 = 0x110;
const LAST_POINTER_BUTTON: u16 = 0x11f;
const MAX_READ_EVENTS: usize = 32;

#[repr(C)]
#[derive(Clone, Copy)]
struct EvdevRaw {
    timestamp: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

static NEXT_EVDEV_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(super) struct EvdevSource {
    file: File,
}

fn permitted_device_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    path.parent() == Some(Path::new("/dev/input"))
        && name
            .strip_prefix("event")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

fn decode(raw: EvdevRaw, sequence: u64) -> Option<PhysicalInputEvent> {
    if raw.timestamp.tv_sec < 0 || !(0..1_000_000).contains(&raw.timestamp.tv_usec) {
        return None;
    }
    let timestamp_usec = (raw.timestamp.tv_sec as u64)
        .checked_mul(1_000_000)?
        .checked_add(raw.timestamp.tv_usec as u64)?;
    let kind = match (raw.kind, raw.code, raw.value) {
        (EV_REL, REL_X, delta) => PhysicalInputKind::RelativeX { delta },
        (EV_REL, REL_Y, delta) => PhysicalInputKind::RelativeY { delta },
        (EV_KEY, FIRST_POINTER_BUTTON..=LAST_POINTER_BUTTON, state @ 0..=1) => {
            PhysicalInputKind::Button { code: u32::from(raw.code), pressed: state != 0 }
        }
        (EV_KEY, code, state @ 0..=1) if code <= 767 => {
            PhysicalInputKind::Key { code: u32::from(code), pressed: state != 0 }
        }
        _ => return None,
    };
    let event = PhysicalInputEvent { sequence, timestamp_usec, kind };
    event.validate().ok()?;
    Some(event)
}

impl EvdevSource {
    pub(super) fn open(path: &Path) -> Result<Self> {
        if !permitted_device_path(path) {
            bail!("physical input device must be explicitly selected as /dev/input/eventN");
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .with_context(|| format!("failed to open physical input {}", path.display()))?;
        Ok(Self { file })
    }

    pub(super) fn read_events(&mut self) -> Result<Vec<PhysicalInputEvent>> {
        let mut bytes = [0u8; std::mem::size_of::<EvdevRaw>() * MAX_READ_EVENTS];
        // SAFETY: bytes is writable, the descriptor is an opened evdev node.
        let len =
            unsafe { libc::read(self.file.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if len < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock
                || error.kind() == io::ErrorKind::Interrupted
            {
                return Ok(Vec::new());
            }
            return Err(error).context("evdev read failed");
        }
        if len == 0 {
            bail!("evdev device closed");
        }
        let len = usize::try_from(len)?;
        let record = std::mem::size_of::<EvdevRaw>();
        if len % record != 0 {
            bail!("evdev returned a partial input_event record");
        }
        let mut events = Vec::new();
        for data in bytes[..len].chunks_exact(record) {
            // SAFETY: data is exactly the native input_event representation.
            let raw = unsafe { (data.as_ptr().cast::<EvdevRaw>()).read_unaligned() };
            let sequence = NEXT_EVDEV_SEQUENCE.fetch_add(1, Ordering::AcqRel);
            if sequence == 0 || sequence == u64::MAX {
                bail!("physical input sequence exhausted");
            }
            if let Some(event) = decode(raw, sequence) {
                events.push(event);
            }
        }
        Ok(events)
    }
}

fn submit_to_waylandd(event: PhysicalInputEvent) -> Result<()> {
    let mut stream = connect_service_socket(ServiceRole::Waylandd)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    let message = IpcEnvelope::new(
        ServiceRole::Displayd,
        ServiceRole::Waylandd,
        MessageKind::WaylandCommand(WaylandCommand::InjectPhysicalInput { event: event.clone() }),
    );
    send_ipc_envelope(&mut stream, &message)?;
    let response: IpcEnvelope = {
        let mut reader = io::BufReader::new(stream);
        read_ipc_envelope(&mut reader)?
    };
    match response.kind {
        MessageKind::WaylandEvent(WaylandEvent::PhysicalInputQueued { sequence })
            if sequence == event.sequence =>
        {
            Ok(())
        }
        MessageKind::WaylandEvent(WaylandEvent::Rejected { reason }) => {
            bail!("input not routed: {reason}")
        }
        _ => bail!("input route acknowledgement mismatch"),
    }
}

pub(super) fn run(mut source: EvdevSource) -> Result<()> {
    loop {
        for event in source.read_events()? {
            // No event persistence/replay. Lost compositor connections discard
            // the input sample instead of replaying stale clicks or keystrokes.
            if let Err(error) = submit_to_waylandd(event) {
                eprintln!("service=displayd op=physical_input event=discarded reason={error:#}");
            }
        }
        thread::sleep(Duration::from_millis(4));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kind: u16, code: u16, value: i32) -> EvdevRaw {
        EvdevRaw { timestamp: libc::timeval { tv_sec: 3, tv_usec: 125 }, kind, code, value }
    }

    #[test]
    fn event_device_path_is_explicit_and_bounded() {
        assert!(permitted_device_path(Path::new("/dev/input/event12")));
        assert!(!permitted_device_path(Path::new("/dev/input/mice")));
        assert!(!permitted_device_path(Path::new("/dev/input/event../shadow")));
        assert!(!permitted_device_path(Path::new("/tmp/event12")));
    }

    #[test]
    fn translates_real_evdev_relative_and_key_events() {
        let x = decode(raw(EV_REL, REL_X, -11), 1).unwrap();
        assert_eq!(x.kind, PhysicalInputKind::RelativeX { delta: -11 });
        assert_eq!(x.timestamp_usec, 3_000_125);
        assert_eq!(
            decode(raw(EV_KEY, 30, 1), 2).unwrap().kind,
            PhysicalInputKind::Key { code: 30, pressed: true }
        );
        assert_eq!(
            decode(raw(EV_KEY, FIRST_POINTER_BUTTON, 1), 3).unwrap().kind,
            PhysicalInputKind::Button { code: 0x110, pressed: true }
        );
    }

    #[test]
    fn malformed_and_unknown_events_fail_closed() {
        assert!(decode(raw(0xffff, 1, 1), 1).is_none());
        assert!(decode(raw(EV_KEY, 30, 7), 1).is_none());
        assert!(decode(raw(EV_REL, REL_X, i32::MIN), 1).is_none());
        assert!(decode(raw(EV_KEY, 800, 1), 1).is_none());
    }
}
