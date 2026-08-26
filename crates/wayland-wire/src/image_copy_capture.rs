use crate::{screencopy::CaptureScratch, WaylandObjectId};
use std::collections::HashMap;

#[derive(Default)]
pub struct ImageCopyCaptureManager {
    pub sessions: HashMap<WaylandObjectId, ImageCopyCaptureSessionState>,
    pub frames: HashMap<WaylandObjectId, ImageCopyCaptureFrameState>,
}

#[derive(Debug, Clone)]
pub struct ImageCopyCaptureSessionState {
    pub source_type: u32,
    pub source_id: u32,
    pub scratch: CaptureScratch,
}

#[derive(Debug, Clone)]
pub struct ImageCopyCaptureFrameState {
    pub session_id: WaylandObjectId,
    pub reuse_hint: usize,
}

impl ImageCopyCaptureManager {
    pub fn new() -> Self {
        Self { sessions: HashMap::new(), frames: HashMap::new() }
    }

    pub fn create_session(&mut self, id: WaylandObjectId, source_type: u32, source_id: u32) {
        self.sessions.insert(
            id,
            ImageCopyCaptureSessionState {
                source_type,
                source_id,
                scratch: CaptureScratch::default(),
            },
        );
    }

    pub fn destroy_session(&mut self, id: WaylandObjectId) {
        self.sessions.remove(&id);
    }

    pub fn create_frame(&mut self, id: WaylandObjectId, session_id: WaylandObjectId) {
        self.frames.insert(id, ImageCopyCaptureFrameState { session_id, reuse_hint: 0 });
    }

    pub fn destroy_frame(&mut self, id: WaylandObjectId) {
        self.frames.remove(&id);
    }
}
