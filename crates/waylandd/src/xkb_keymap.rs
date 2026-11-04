//! Production Wayland wl_keyboard.keymap FD transport using libxkbcommon.
//! The keymap is generated outside the client and sealed before SCM_RIGHTS.
use anyhow::{Context, Result, bail};
use std::{
    ffi::{CStr, CString},
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::net::UnixStream,
};
use wayland_wire::{WaylandMessage, WaylandObjectId, WaylandOpcode};

#[repr(C)]
struct XkbRuleNames {
    rules: *const libc::c_char,
    model: *const libc::c_char,
    layout: *const libc::c_char,
    variant: *const libc::c_char,
    options: *const libc::c_char,
}

type ContextNew = unsafe extern "C" fn(u32) -> *mut libc::c_void;
type ContextUnref = unsafe extern "C" fn(*mut libc::c_void);
type KeymapNew = unsafe extern "C" fn(
    *mut libc::c_void, *const XkbRuleNames, u32,
) -> *mut libc::c_void;
type KeymapUnref = unsafe extern "C" fn(*mut libc::c_void);
type KeymapText = unsafe extern "C" fn(*mut libc::c_void, u32) -> *mut libc::c_char;

unsafe fn symbol<T: Copy>(library: *mut libc::c_void, name: &[u8]) -> Result<T> {
    let cname = CStr::from_bytes_with_nul(name)?;
    // SAFETY: library is live; signatures below match libxkbcommon.so ABI.
    let raw = unsafe { libc::dlsym(library, cname.as_ptr()) };
    if raw.is_null() { bail!("missing libxkbcommon symbol {}", cname.to_string_lossy()); }
    Ok(unsafe { std::mem::transmute_copy::<*mut libc::c_void, T>(&raw) })
}

pub(super) fn generate_keymap() -> Result<Vec<u8>> {
    // SAFETY: constant NUL-terminated soname.
    let library = unsafe { libc::dlopen(c"libxkbcommon.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if library.is_null() { bail!("physical keyboard requires libxkbcommon.so.0"); }
    let result = (|| -> Result<Vec<u8>> {
        // SAFETY: symbols are from loaded libxkbcommon with the signatures above.
        unsafe {
            let context_new: ContextNew = symbol(library, b"xkb_context_new\0")?;
            let context_unref: ContextUnref = symbol(library, b"xkb_context_unref\0")?;
            let keymap_new: KeymapNew = symbol(library, b"xkb_keymap_new_from_names\0")?;
            let keymap_unref: KeymapUnref = symbol(library, b"xkb_keymap_unref\0")?;
            let keymap_text: KeymapText = symbol(library, b"xkb_keymap_get_as_string\0")?;
            let context = context_new(0);
            if context.is_null() { bail!("xkb_context_new failed"); }
            let result = (|| -> Result<Vec<u8>> {
                let layout_name = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_else(|_| "us".into());
                let layout = CString::new(layout_name).context("XKB_DEFAULT_LAYOUT contains NUL")?;
                let names = XkbRuleNames {
                    rules: std::ptr::null(),
                    model: std::ptr::null(),
                    layout: layout.as_ptr(),
                    variant: std::ptr::null(),
                    options: std::ptr::null(),
                };
                let keymap = keymap_new(context, &names, 0);
                if keymap.is_null() { bail!("xkb_keymap_new_from_names failed"); }
                let text_result = (|| -> Result<Vec<u8>> {
                    let data = keymap_text(keymap, 1); // XKB_KEYMAP_FORMAT_TEXT_V1
                    if data.is_null() { bail!("xkb_keymap_get_as_string failed"); }
                    let text = CStr::from_ptr(data).to_bytes_with_nul().to_vec();
                    libc::free(data.cast());
                    if text.len() < 16 || text.len() > 1024 * 1024 {
                        bail!("XKB keymap size outside budget");
                    }
                    Ok(text)
                })();
                keymap_unref(keymap);
                text_result
            })();
            context_unref(context);
            result
        }
    })();
    // SAFETY: all xkb allocations have been released.
    unsafe { libc::dlclose(library) };
    result
}

pub(super) fn send_keymap(stream: &UnixStream, keyboard: WaylandObjectId) -> Result<()> {
    let map = generate_keymap()?;
    let size = u32::try_from(map.len())?;
    // SAFETY: memfd_create accepts a NUL-terminated name and flags.
    let fd = unsafe {
        libc::memfd_create(c"tuff-xwin-keymap".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
    };
    if fd < 0 { return Err(std::io::Error::last_os_error()).context("memfd_create keymap"); }
    // SAFETY: ownership transferred exactly once from memfd_create.
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(&map)?;
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    // SAFETY: backing is no longer writable after sealing.
    if unsafe {
        libc::fcntl(fd, libc::F_ADD_SEALS,
                    libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL)
    } < 0 {
        bail!("sealing XKB keymap memfd failed: {}", std::io::Error::last_os_error());
    }
    let mut payload = Vec::with_capacity(8);
    payload.extend_from_slice(&1_u32.to_le_bytes()); // XKB_KEYMAP_FORMAT_TEXT_V1
    payload.extend_from_slice(&size.to_le_bytes());
    let encoded = wayland_wire::codec::encode_message(
        &WaylandMessage::new(keyboard, WaylandOpcode(0), payload),
    )?;
    let sent = wayland_wire::fd::send_with_fds(stream, &encoded, &[file.as_raw_fd()])?;
    if sent == 0 { bail!("keymap SCM_RIGHTS send returned zero"); }
    if sent < encoded.len() {
        let mut socket = stream;
        socket.write_all(&encoded[sent..])?;
    }
    // wl_keyboard.repeat_info, supported by our advertised version 7.
    let mut repeat = Vec::with_capacity(8);
    repeat.extend_from_slice(&25_i32.to_le_bytes());
    repeat.extend_from_slice(&600_i32.to_le_bytes());
    let encoded = wayland_wire::codec::encode_message(
        &WaylandMessage::new(keyboard, WaylandOpcode(5), repeat),
    )?;
    let mut socket = stream;
    socket.write_all(&encoded)?;
    Ok(())
}
