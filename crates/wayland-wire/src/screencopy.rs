use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;
use std::slice;

#[derive(Debug, Clone, Default)]
pub struct CaptureScratch {
    pixels: Vec<u32>,
}

impl CaptureScratch {
    pub fn prepare_pixels(&mut self, pixel_count: usize) -> &mut [u32] {
        if self.pixels.capacity() < pixel_count {
            self.pixels.reserve_exact(pixel_count - self.pixels.capacity());
        }

        self.pixels.resize(pixel_count, 0);
        &mut self.pixels
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.pixels.as_ptr() as *const u8, self.pixels.len() * 4) }
    }

    pub fn clear(&mut self) {
        self.pixels.clear();
    }

    pub fn capacity(&self) -> usize {
        self.pixels.capacity()
    }
}

#[derive(Default)]
pub struct ScreencopyManager {
    pub frames: HashMap<WaylandObjectId, ScreencopyFrameState>,
    pub scratch: CaptureScratch,
}

#[derive(Debug, Clone)]
pub struct ScreencopyFrameState {
    pub output_id: Option<WaylandObjectId>,
    pub overlay_cursor: bool,
    pub region: Option<(i32, i32, i32, i32)>,
    pub buffer_id: Option<WaylandObjectId>,
    pub copied: bool,
    pub scratch: CaptureScratch,
}

impl ScreencopyManager {
    pub fn new() -> Self {
        Self { frames: HashMap::new(), scratch: CaptureScratch::default() }
    }

    pub fn capture_output(
        &mut self,
        id: WaylandObjectId,
        output_id: Option<WaylandObjectId>,
        overlay_cursor: bool,
    ) {
        self.frames.insert(
            id,
            ScreencopyFrameState {
                output_id,
                overlay_cursor,
                region: None,
                buffer_id: None,
                copied: false,
                scratch: CaptureScratch::default(),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_output_region(
        &mut self,
        id: WaylandObjectId,
        output_id: Option<WaylandObjectId>,
        overlay_cursor: bool,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<()> {
        if w <= 0 || h <= 0 {
            return Err(WireError::ProtocolError("invalid capture region".into()));
        }
        self.frames.insert(
            id,
            ScreencopyFrameState {
                output_id,
                overlay_cursor,
                region: Some((x, y, w, h)),
                buffer_id: None,
                copied: false,
                scratch: CaptureScratch::default(),
            },
        );
        Ok(())
    }

    pub fn destroy(&mut self, id: WaylandObjectId) {
        self.frames.remove(&id);
    }
}
