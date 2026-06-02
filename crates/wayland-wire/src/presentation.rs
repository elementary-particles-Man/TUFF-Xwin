use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait PresentationClock: Send + Sync {
    fn now_nsec(&self) -> u64;
}

pub struct SystemPresentationClock;
impl PresentationClock for SystemPresentationClock {
    fn now_nsec(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
    }
}

pub struct PresentationManager {
    pub feedbacks: HashMap<WaylandObjectId, PresentationFeedbackState>,
}

pub struct PresentationFeedbackState {
    pub surface_id: WaylandObjectId,
}

impl PresentationManager {
    pub fn new() -> Self {
        Self { feedbacks: HashMap::new() }
    }

    pub fn feedback(&mut self, id: WaylandObjectId, surface_id: WaylandObjectId) {
        self.feedbacks.insert(id, PresentationFeedbackState { surface_id });
    }

    pub fn destroy(&mut self, id: WaylandObjectId) {
        self.feedbacks.remove(&id);
    }
}
