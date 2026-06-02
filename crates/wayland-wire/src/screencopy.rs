use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

pub struct ScreencopyManager {
    pub frames: HashMap<WaylandObjectId, ScreencopyFrameState>,
}

#[derive(Debug, Clone)]
pub struct ScreencopyFrameState {
    pub output_id: Option<WaylandObjectId>,
    pub overlay_cursor: bool,
    pub region: Option<(i32, i32, i32, i32)>,
    pub buffer_id: Option<WaylandObjectId>,
    pub copied: bool,
}

impl ScreencopyManager {
    pub fn new() -> Self {
        Self { frames: HashMap::new() }
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
            },
        );
    }

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
            },
        );
        Ok(())
    }

    pub fn destroy(&mut self, id: WaylandObjectId) {
        self.frames.remove(&id);
    }
}
