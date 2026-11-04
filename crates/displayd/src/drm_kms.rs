//! Explicit Linux DRM/KMS scanout backend. No mock publication is used on this path.
//! Only the selected primary DRM device is opened. The caller must already own DRM master.
//! Legacy modesetting and dumb scanout are used as a baseline; no GPU renderer is required.
use super::{
    BackendOutputEvent, DisplayBackend, FramePublication, MAX_OUTPUT_FRAMEBUFFER_BYTES,
    MAX_OUTPUTS, OutputGeometry, OutputMode, PresentationCadence, PresentationToken,
    WL_SHM_FORMAT_XRGB8888,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::CStr,
    fs::{File, OpenOptions},
    os::fd::AsRawFd,
    path::Path,
    ptr,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

// Page flip userdata is an opaque monotonically increasing identifier, never
// a pointer to temporary memory. Late events cannot dereference freed memory.
static NEXT_FLIP_ID: AtomicU64 = AtomicU64::new(1);
static LAST_FLIP_ID: AtomicU64 = AtomicU64::new(0);
unsafe extern "C" fn drm_page_flip_done(
    _fd: libc::c_int, _sequence: u32, _sec: u32, _usec: u32,
    user_data: *mut libc::c_void,
) {
    LAST_FLIP_ID.store(user_data as usize as u64, Ordering::Release);
}

#[repr(C)]
struct DrmEventContext {
    version: libc::c_int,
    vblank_handler: Option<unsafe extern "C" fn(libc::c_int, u32, u32, u32, *mut libc::c_void)>,
    page_flip_handler: Option<unsafe extern "C" fn(libc::c_int, u32, u32, u32, *mut libc::c_void)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DrmModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    mode_type: u32,
    name: [libc::c_char; 32],
}

#[repr(C)]
struct DrmResources {
    count_fbs: libc::c_int,
    fbs: *mut u32,
    count_crtcs: libc::c_int,
    crtcs: *mut u32,
    count_connectors: libc::c_int,
    connectors: *mut u32,
    count_encoders: libc::c_int,
    encoders: *mut u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
struct DrmConnector {
    connector_id: u32,
    encoder_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: libc::c_int,
    mm_width: u32,
    mm_height: u32,
    subpixel: libc::c_int,
    count_modes: libc::c_int,
    modes: *mut DrmModeInfo,
    count_props: libc::c_int,
    props: *mut u32,
    prop_values: *mut u64,
    count_encoders: libc::c_int,
    encoders: *mut u32,
}

#[repr(C)]
struct DrmEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

type GetResources = unsafe extern "C" fn(libc::c_int) -> *mut DrmResources;
type FreeResources = unsafe extern "C" fn(*mut DrmResources);
type GetConnector = unsafe extern "C" fn(libc::c_int, u32) -> *mut DrmConnector;
type FreeConnector = unsafe extern "C" fn(*mut DrmConnector);
type GetEncoder = unsafe extern "C" fn(libc::c_int, u32) -> *mut DrmEncoder;
type FreeEncoder = unsafe extern "C" fn(*mut DrmEncoder);
type CreateDumb = unsafe extern "C" fn(
    libc::c_int,
    u32,
    u32,
    u32,
    u32,
    *mut u32,
    *mut u32,
    *mut u64,
) -> libc::c_int;
type MapDumb = unsafe extern "C" fn(libc::c_int, u32, *mut u64) -> libc::c_int;
type DestroyDumb = unsafe extern "C" fn(libc::c_int, u32) -> libc::c_int;
type AddFb = unsafe extern "C" fn(libc::c_int, u32, u32, u8, u8, u32, u32, *mut u32) -> libc::c_int;
type RmFb = unsafe extern "C" fn(libc::c_int, u32) -> libc::c_int;
type SetCrtc = unsafe extern "C" fn(
    libc::c_int,
    u32,
    u32,
    u32,
    u32,
    *const u32,
    libc::c_int,
    *const DrmModeInfo,
) -> libc::c_int;
type PageFlip = unsafe extern "C" fn(
    libc::c_int, u32, u32, u32, *mut libc::c_void,
) -> libc::c_int;
type HandleEvent = unsafe extern "C" fn(libc::c_int, *mut DrmEventContext) -> libc::c_int;

struct DrmApi {
    library: *mut libc::c_void,
    get_resources: GetResources,
    free_resources: FreeResources,
    get_connector: GetConnector,
    free_connector: FreeConnector,
    get_encoder: GetEncoder,
    free_encoder: FreeEncoder,
    create_dumb: CreateDumb,
    map_dumb: MapDumb,
    destroy_dumb: DestroyDumb,
    add_fb: AddFb,
    rm_fb: RmFb,
    set_crtc: SetCrtc,
    page_flip: PageFlip,
    handle_event: HandleEvent,
}

unsafe fn symbol<T: Copy>(library: *mut libc::c_void, name: &[u8]) -> Result<T> {
    let c_name = CStr::from_bytes_with_nul(name)?;
    // SAFETY: the library remains loaded for the lifetime of the API and the
    // declared signatures correspond to libdrm's public C ABI.
    let address = unsafe { libc::dlsym(library, c_name.as_ptr()) };
    if address.is_null() {
        bail!("libdrm is missing required symbol {}", c_name.to_string_lossy());
    }
    // SAFETY: POSIX dlsym returns a function address, all target types are
    // function pointers with the platform's C ABI.
    Ok(unsafe { std::mem::transmute_copy::<*mut libc::c_void, T>(&address) })
}

impl DrmApi {
    fn load() -> Result<Self> {
        // SAFETY: a NUL-terminated library name is passed to the runtime loader.
        let library =
            unsafe { libc::dlopen(c"libdrm.so.2".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if library.is_null() {
            bail!("drm-kms requires the system runtime library libdrm.so.2");
        }
        let loaded = (|| -> Result<Self> {
            // SAFETY: signatures are declared above from xf86drmMode.h.
            unsafe {
                Ok(Self {
                    library,
                    get_resources: symbol(library, b"drmModeGetResources\0")?,
                    free_resources: symbol(library, b"drmModeFreeResources\0")?,
                    get_connector: symbol(library, b"drmModeGetConnector\0")?,
                    free_connector: symbol(library, b"drmModeFreeConnector\0")?,
                    get_encoder: symbol(library, b"drmModeGetEncoder\0")?,
                    free_encoder: symbol(library, b"drmModeFreeEncoder\0")?,
                    create_dumb: symbol(library, b"drmModeCreateDumbBuffer\0")?,
                    map_dumb: symbol(library, b"drmModeMapDumbBuffer\0")?,
                    destroy_dumb: symbol(library, b"drmModeDestroyDumbBuffer\0")?,
                    add_fb: symbol(library, b"drmModeAddFB\0")?,
                    rm_fb: symbol(library, b"drmModeRmFB\0")?,
                    set_crtc: symbol(library, b"drmModeSetCrtc\0")?,
                    page_flip: symbol(library, b"drmModePageFlip\0")?,
                    handle_event: symbol(library, b"drmHandleEvent\0")?,
                })
            }
        })();
        if loaded.is_err() {
            // SAFETY: the API did not acquire ownership on failure.
            unsafe { libc::dlclose(library) };
        }
        loaded
    }
}

impl Drop for DrmApi {
    fn drop(&mut self) {
        // SAFETY: all DRM objects are released by DrmKmsBackend before the library.
        unsafe { libc::dlclose(self.library) };
    }
}

struct ScanoutBuffer {
    handle: u32,
    framebuffer_id: u32,
    pitch: u32,
    bytes: usize,
    mapping: *mut libc::c_void,
}

struct KmsOutput {
    connector_id: u32,
    crtc_id: u32,
    mode: DrmModeInfo,
    geometry: OutputGeometry,
    refresh_hz: u32,
    buffers: [Option<ScanoutBuffer>; 2],
    active_slot: Option<usize>,
    last_frame: Option<FramePublication>,
}

pub(super) struct DrmKmsBackend {
    device: File,
    api: DrmApi,
    outputs: BTreeMap<String, KmsOutput>,
    events: VecDeque<BackendOutputEvent>,
    backend_instance_id: u64,
    next_token_sequence: u64,
}

fn select_mode(connector: &DrmConnector) -> Option<DrmModeInfo> {
    if connector.count_modes <= 0 || connector.modes.is_null() || connector.count_modes > 256 {
        return None;
    }
    let count = usize::try_from(connector.count_modes).ok()?;
    // SAFETY: libdrm owns count_modes entries until drmModeFreeConnector.
    let modes = unsafe { std::slice::from_raw_parts(connector.modes, count) };
    modes
        .iter()
        .find(|mode| mode.mode_type & (1 << 3) != 0 && mode.hdisplay != 0 && mode.vdisplay != 0)
        .or_else(|| modes.iter().find(|mode| mode.hdisplay != 0 && mode.vdisplay != 0))
        .copied()
}

fn validate_frame(request: &FramePublication, output: &KmsOutput) -> Result<usize> {
    let width = usize::from(output.mode.hdisplay);
    let height = usize::from(output.mode.vdisplay);
    if request.output_id != output.geometry.output_id
        || request.geometry.width != u32::from(output.mode.hdisplay)
        || request.geometry.height != u32::from(output.mode.vdisplay)
        || request.format != WL_SHM_FORMAT_XRGB8888
        || request.stride != request.geometry.width.checked_mul(4).context("stride overflow")?
    {
        bail!("drm-kms frame geometry, format, or output identity mismatch");
    }
    let count = width.checked_mul(height).context("drm-kms pixel count overflow")?;
    let size = count.checked_mul(4).context("drm-kms byte count overflow")?;
    if request.pixels.len() != count || size > MAX_OUTPUT_FRAMEBUFFER_BYTES {
        bail!("drm-kms frame pixel payload invalid or over budget");
    }
    Ok(size)
}

impl DrmKmsBackend {
    pub(super) fn open(device_path: &Path, epoch: u64) -> Result<Self> {
        if !device_path.is_absolute() || !device_path.to_string_lossy().starts_with("/dev/dri/card")
        {
            bail!("drm-kms requires an explicit absolute /dev/dri/cardN device");
        }
        let device =
            OpenOptions::new().read(true).write(true).open(device_path).with_context(|| {
                format!("cannot open DRM primary device {}", device_path.display())
            })?;
        let api = DrmApi::load()?;
        let fd = device.as_raw_fd();
        // SAFETY: libdrm owns resources and they are freed before return.
        let resources = unsafe { (api.get_resources)(fd) };
        if resources.is_null() {
            bail!("drmModeGetResources failed: {}", std::io::Error::last_os_error());
        }
        let result = Self::discover(&api, fd, resources, epoch);
        // SAFETY: resources came from drmModeGetResources.
        unsafe { (api.free_resources)(resources) };
        let (outputs, events) = result?;
        if outputs.is_empty() {
            bail!("drm-kms found no connected connector with a compatible free CRTC");
        }
        Ok(Self {
            device,
            api,
            outputs,
            events,
            backend_instance_id: epoch,
            next_token_sequence: 1,
        })
    }

    fn discover(
        api: &DrmApi,
        fd: libc::c_int,
        resources: *mut DrmResources,
        epoch: u64,
    ) -> Result<(BTreeMap<String, KmsOutput>, VecDeque<BackendOutputEvent>)> {
        // SAFETY: resources are valid for the duration of this method.
        let res = unsafe { &*resources };
        if res.count_connectors < 0
            || res.count_crtcs < 0
            || res.count_connectors > 256
            || res.count_crtcs > 32
            || (res.count_connectors > 0 && res.connectors.is_null())
            || (res.count_crtcs > 0 && res.crtcs.is_null())
        {
            bail!("drm-kms resource table invalid or exceeds bounds");
        }
        // SAFETY: libdrm supplies these arrays with their declared counts.
        let connectors =
            unsafe { std::slice::from_raw_parts(res.connectors, res.count_connectors as usize) };
        let crtcs = unsafe { std::slice::from_raw_parts(res.crtcs, res.count_crtcs as usize) };
        let mut outputs = BTreeMap::new();
        let mut events = VecDeque::new();
        let mut used_crtcs = BTreeSet::new();
        let mut origin_x = 0_i32;
        let mut sequence = 1_u64;

        for &connector_id in connectors {
            if outputs.len() >= MAX_OUTPUTS {
                break;
            }
            // SAFETY: connector pointer is freed after all its fields are read.
            let pointer = unsafe { (api.get_connector)(fd, connector_id) };
            if pointer.is_null() {
                continue;
            }
            let discovered = (|| -> Option<(u32, DrmModeInfo)> {
                // SAFETY: non-null pointer returned by libdrm.
                let conn = unsafe { &*pointer };
                if conn.connection != 1 {
                    return None;
                } // DRM_MODE_CONNECTED
                let mode = select_mode(conn)?;
                if conn.count_encoders < 0
                    || conn.count_encoders > 256
                    || (conn.count_encoders > 0 && conn.encoders.is_null())
                {
                    return None;
                }
                // SAFETY: libdrm owns the encoder array.
                let encoders = unsafe {
                    std::slice::from_raw_parts(conn.encoders, conn.count_encoders as usize)
                };
                let candidates = std::iter::once(conn.encoder_id)
                    .chain(encoders.iter().copied())
                    .filter(|id| *id != 0);
                for encoder_id in candidates {
                    // SAFETY: get/free encoder pair.
                    let ep = unsafe { (api.get_encoder)(fd, encoder_id) };
                    if ep.is_null() {
                        continue;
                    }
                    // SAFETY: ep is live until freed below.
                    let available = unsafe { &*ep };
                    let selected = crtcs.iter().enumerate().find_map(|(index, crtc_id)| {
                        let bit = 1u32.checked_shl(index as u32)?;
                        ((available.possible_crtcs & bit) != 0 && !used_crtcs.contains(crtc_id))
                            .then_some(*crtc_id)
                    });
                    // SAFETY: ep came from drmModeGetEncoder.
                    unsafe { (api.free_encoder)(ep) };
                    if let Some(crtc) = selected {
                        return Some((crtc, mode));
                    }
                }
                None
            })();
            // SAFETY: pointer came from drmModeGetConnector.
            unsafe { (api.free_connector)(pointer) };
            let Some((crtc_id, mode)) = discovered else {
                continue;
            };
            let width = u32::from(mode.hdisplay);
            let height = u32::from(mode.vdisplay);
            let Some(stride) = width.checked_mul(4) else { continue };
            let Some(bytes) = u64::from(stride).checked_mul(u64::from(height)) else { continue };
            if bytes > MAX_OUTPUT_FRAMEBUFFER_BYTES as u64 {
                continue;
            }
            let output_id = format!("drm-connector-{connector_id}");
            let geometry = OutputGeometry {
                output_id: output_id.clone(),
                width,
                height,
                stride,
                format: WL_SHM_FORMAT_XRGB8888,
                origin_x,
                origin_y: 0,
                output_generation: 1,
            };
            let refresh_hz = mode.vrefresh.max(1);
            events.push_back(BackendOutputEvent::Connected {
                backend_instance_id: epoch,
                backend_output_id: output_id.clone(),
                event_sequence: sequence,
                geometry: geometry.clone(),
                cadence: PresentationCadence {
                    period_ns: 1_000_000_000_u64 / u64::from(refresh_hz),
                },
            });
            sequence += 1;
            used_crtcs.insert(crtc_id);
            outputs.insert(
                output_id,
                KmsOutput {
                    connector_id,
                    crtc_id,
                    mode,
                    geometry,
                    refresh_hz,
                    buffers: [None, None],
                    active_slot: None,
                    last_frame: None,
                },
            );
            origin_x = origin_x.saturating_add(i32::from(mode.hdisplay));
        }
        Ok((outputs, events))
    }

    fn create_buffer(&self, output: &KmsOutput) -> Result<ScanoutBuffer> {
        let fd = self.device.as_raw_fd();
        let width = u32::from(output.mode.hdisplay);
        let height = u32::from(output.mode.vdisplay);
        let (mut handle, mut pitch, mut size) = (0_u32, 0_u32, 0_u64);
        // SAFETY: output pointers refer to writable stack variables.
        if unsafe {
            (self.api.create_dumb)(fd, width, height, 32, 0, &mut handle, &mut pitch, &mut size)
        } != 0
        {
            bail!("drmModeCreateDumbBuffer failed: {}", std::io::Error::last_os_error());
        }
        let allocated = (|| -> Result<ScanoutBuffer> {
            let row = width.checked_mul(4).context("drm scanout row overflow")?;
            if pitch < row
                || size < u64::from(pitch) * u64::from(height)
                || size > MAX_OUTPUT_FRAMEBUFFER_BYTES as u64
            {
                bail!("drm dumb buffer pitch/size invalid");
            }
            let mut framebuffer_id = 0_u32;
            // SAFETY: handle belongs to fd and framebuffer_id is writable.
            if unsafe {
                (self.api.add_fb)(fd, width, height, 24, 32, pitch, handle, &mut framebuffer_id)
            } != 0
            {
                bail!("drmModeAddFB failed: {}", std::io::Error::last_os_error());
            }
            let mapped = (|| -> Result<ScanoutBuffer> {
                let mut offset = 0_u64;
                // SAFETY: offset is writable.
                if unsafe { (self.api.map_dumb)(fd, handle, &mut offset) } != 0 {
                    bail!("drmModeMapDumbBuffer failed: {}", std::io::Error::last_os_error());
                }
                let len = usize::try_from(size)?;
                let offset = libc::off_t::try_from(offset)?;
                // SAFETY: DRM driver provided an offset/size for this handle.
                let mapping = unsafe {
                    libc::mmap(
                        ptr::null_mut(),
                        len,
                        libc::PROT_READ | libc::PROT_WRITE,
                        libc::MAP_SHARED,
                        fd,
                        offset,
                    )
                };
                if mapping == libc::MAP_FAILED {
                    bail!("drm scanout mmap failed: {}", std::io::Error::last_os_error());
                }
                // SAFETY: mmap succeeded and the memory is mapped for writing.
                unsafe { ptr::write_bytes(mapping.cast::<u8>(), 0, len) };
                Ok(ScanoutBuffer { handle, framebuffer_id, pitch, bytes: len, mapping })
            })();
            if mapped.is_err() {
                // SAFETY: framebuffer_id was successfully registered above.
                unsafe { (self.api.rm_fb)(fd, framebuffer_id) };
            }
            mapped
        })();
        if allocated.is_err() {
            // SAFETY: handle was successfully allocated above.
            unsafe { (self.api.destroy_dumb)(fd, handle) };
        }
        allocated
    }

    fn destroy_buffer(&self, buffer: ScanoutBuffer) {
        let fd = self.device.as_raw_fd();
        // SAFETY: mapping and framebuffer belong to this backend and remain live.
        unsafe {
            libc::munmap(buffer.mapping, buffer.bytes);
            (self.api.rm_fb)(fd, buffer.framebuffer_id);
            (self.api.destroy_dumb)(fd, buffer.handle);
        }
    }

    fn scanout(&mut self, frame: FramePublication) -> Result<()> {
        let (next, needs_allocation) = {
            let output = self.outputs.get(&frame.output_id).context("unknown DRM output")?;
            validate_frame(&frame, output)?;
            let slot = output.active_slot.map_or(0, |active| 1 - active);
            (slot, output.buffers[slot].is_none())
        };
        if needs_allocation {
            let new_buffer = self
                .create_buffer(self.outputs.get(&frame.output_id).context("unknown DRM output")?)?;
            self.outputs.get_mut(&frame.output_id).context("unknown DRM output")?.buffers[next] =
                Some(new_buffer);
        }
        let output = self.outputs.get_mut(&frame.output_id).context("unknown DRM output")?;
        let buffer = output.buffers[next].as_mut().context("DRM scanout not allocated")?;
        let width = usize::try_from(output.geometry.width)?;
        let height = usize::try_from(output.geometry.height)?;
        let pitch = usize::try_from(buffer.pitch)?;
        let required = pitch.checked_mul(height).context("DRM pitch overflow")?;
        if required > buffer.bytes {
            bail!("DRM mapped buffer too small");
        }
        for row in 0..height {
            // SAFETY: validate_frame guarantees exactly width*height pixels;
            // checked pitch/height bounds guarantee the mapped destination.
            unsafe {
                ptr::copy_nonoverlapping(
                    frame.pixels.as_ptr().add(row * width).cast::<u8>(),
                    buffer.mapping.cast::<u8>().add(row * pitch),
                    width * 4,
                );
            }
        }
        let first_modeset = output.active_slot.is_none();
        let crtc = output.crtc_id;
        let framebuffer_id = buffer.framebuffer_id;
        let connector = output.connector_id;
        let mode = output.mode;
        let fd = self.device.as_raw_fd();
        if first_modeset {
            // SAFETY: all objects are live on this fd, and mode was copied from libdrm.
            let result = unsafe {
                (self.api.set_crtc)(fd, crtc, framebuffer_id, 0, 0, &connector, 1, &mode)
            };
            if result != 0 {
                bail!("drmModeSetCrtc rejected initial frame (check DRM master/VT): {}",
                      std::io::Error::last_os_error());
            }
        } else {
            let flip_id = NEXT_FLIP_ID.fetch_add(1, Ordering::AcqRel);
            if flip_id == 0 || flip_id > usize::MAX as u64 {
                bail!("drm page flip token exhausted");
            }
            // SAFETY: userdata is an integer token (not dereferenced); DRM owns fd/crtc/fb.
            let result = unsafe {
                (self.api.page_flip)(fd, crtc, framebuffer_id, 1, flip_id as usize as *mut libc::c_void)
            };
            if result != 0 {
                bail!("drmModePageFlip rejected frame: {}", std::io::Error::last_os_error());
            }
            let mut ctx = DrmEventContext {
                version: 2, vblank_handler: None,
                page_flip_handler: Some(drm_page_flip_done),
            };
            let deadline = Instant::now() + Duration::from_secs(2);
            while LAST_FLIP_ID.load(Ordering::Acquire) != flip_id {
                let now = Instant::now();
                if now >= deadline { bail!("DRM page flip completion timed out"); }
                let millis = deadline.duration_since(now).as_millis().min(2000) as libc::c_int;
                let mut descriptor = libc::pollfd {
                    fd, events: libc::POLLIN, revents: 0,
                };
                // SAFETY: descriptor is writable for exactly one pollfd.
                let ready = unsafe { libc::poll(&mut descriptor, 1, millis) };
                if ready < 0 {
                    if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    bail!("DRM page flip poll failed: {}", std::io::Error::last_os_error());
                }
                if ready == 0 { bail!("DRM page flip completion timed out"); }
                if descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    bail!("DRM page flip device reported an error");
                }
                // SAFETY: ctx is a valid version-2 drmEventContext and lives across the call.
                if unsafe { (self.api.handle_event)(fd, &mut ctx) } != 0 {
                    bail!("drmHandleEvent failed: {}", std::io::Error::last_os_error());
                }
            }
        }
        output.active_slot = Some(next);
        output.last_frame = Some(frame);
        Ok(())
    }
}

impl Drop for DrmKmsBackend {
    fn drop(&mut self) {
        let mut buffers = Vec::new();
        for output in self.outputs.values_mut() {
            for slot in &mut output.buffers {
                if let Some(buffer) = slot.take() {
                    buffers.push(buffer);
                }
            }
        }
        for buffer in buffers {
            self.destroy_buffer(buffer);
        }
    }
}

impl DisplayBackend for DrmKmsBackend {
    fn poll_output_events(&mut self) -> Result<Vec<BackendOutputEvent>> {
        Ok(self.events.drain(..).collect())
    }
    fn enumerate_outputs(&self) -> Result<Vec<OutputMode>> {
        Ok(self
            .outputs
            .iter()
            .map(|(name, output)| OutputMode {
                name: name.clone(),
                width: output.geometry.width,
                height: output.geometry.height,
                refresh_hz: output.refresh_hz,
            })
            .collect())
    }
    fn set_mode(&mut self, output: &str, mode: &OutputMode) -> Result<()> {
        let current = self.outputs.get(output).context("unknown drm-kms output")?;
        if mode.name != output
            || mode.width != current.geometry.width
            || mode.height != current.geometry.height
            || mode.refresh_hz != current.refresh_hz
        {
            bail!("drm-kms mode selection requires a discovered mode");
        }
        Ok(())
    }
    fn set_gamma(
        &mut self,
        _output: &str,
        _red: &[u16],
        _green: &[u16],
        _blue: &[u16],
    ) -> Result<()> {
        bail!("drm-kms gamma LUT is not supported by this legacy scanout backend")
    }
    fn publish_frame(&mut self, request: FramePublication) -> Result<()> {
        self.scanout(request)
    }
    fn submit_frame(&mut self, request: FramePublication) -> Result<PresentationToken> {
        let token = PresentationToken {
            backend_instance_id: self.backend_instance_id,
            sequence: self.next_token_sequence,
            output_id: request.output_id.clone(),
            output_generation: request.output_generation,
            scene_generation: request.scene_generation,
            frame_id: request.frame_id,
        };
        token.validate().map_err(anyhow::Error::msg)?;
        self.scanout(request)?;
        self.next_token_sequence = self.next_token_sequence.saturating_add(1);
        Ok(token)
    }
    fn submission_completes_immediately(&self) -> bool {
        // Legacy drmModeSetCrtc returns synchronously after the modeset is accepted.
        // This is not a VBlank/page-flip timestamp; presentation feedback is limited.
        true
    }
    fn capture_published(&self, output: &str) -> Result<Option<FramePublication>> {
        Ok(self.outputs.get(output).and_then(|output| output.last_frame.clone()))
    }
    fn reconfigure_output(&mut self, output_id: &str, _generation: u64) -> Result<()> {
        if !self.outputs.contains_key(output_id) {
            bail!("unknown drm-kms output");
        }
        Ok(())
    }
    fn remove_output(&mut self, output_id: &str) {
        if let Some(output) = self.outputs.get_mut(output_id) {
            output.last_frame = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_validation_rejects_non_scanout_format_and_shape() {
        let mode = DrmModeInfo {
            clock: 0,
            hdisplay: 2,
            hsync_start: 0,
            hsync_end: 0,
            htotal: 0,
            hskew: 0,
            vdisplay: 2,
            vsync_start: 0,
            vsync_end: 0,
            vtotal: 0,
            vscan: 0,
            vrefresh: 60,
            flags: 0,
            mode_type: 0,
            name: [0; 32],
        };
        let geometry = OutputGeometry {
            output_id: "drm-connector-1".into(),
            width: 2,
            height: 2,
            stride: 8,
            format: WL_SHM_FORMAT_XRGB8888,
            origin_x: 0,
            origin_y: 0,
            output_generation: 1,
        };
        let output = KmsOutput {
            connector_id: 1,
            crtc_id: 1,
            mode,
            geometry: geometry.clone(),
            refresh_hz: 60,
            buffers: [None, None],
            active_slot: None,
            last_frame: None,
        };
        let pixels: std::sync::Arc<[u32]> = vec![0; 4].into();
        let mut frame = FramePublication {
            frame_id: 1,
            commit_id: 1,
            output_id: geometry.output_id.clone(),
            output_generation: 1,
            scene_epoch: 1,
            scene_generation: 1,
            geometry,
            format: WL_SHM_FORMAT_XRGB8888,
            stride: 8,
            pixels,
            damage: vec![],
        };
        assert_eq!(validate_frame(&frame, &output).unwrap(), 16);
        frame.format = 0;
        assert!(validate_frame(&frame, &output).is_err());
        frame.format = WL_SHM_FORMAT_XRGB8888;
        frame.stride = 4;
        assert!(validate_frame(&frame, &output).is_err());
    }
}
