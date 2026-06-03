use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct IdleInhibitorState {
    pub surface_id: WaylandObjectId,
}

#[derive(Debug, Default)]
pub struct IdleInhibitState {
    pub inhibitors: HashMap<WaylandObjectId, IdleInhibitorState>,
    pub surface_counts: HashMap<WaylandObjectId, usize>,
    pub inhibited: bool,
}

impl IdleInhibitState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_inhibitor(&mut self, inhibitor_id: WaylandObjectId, surface_id: WaylandObjectId) {
        self.inhibitors.insert(inhibitor_id, IdleInhibitorState { surface_id });
        *self.surface_counts.entry(surface_id).or_insert(0) += 1;
        self.inhibited = true;
    }

    pub fn destroy_inhibitor(&mut self, inhibitor_id: WaylandObjectId) -> Result<()> {
        let state = self
            .inhibitors
            .remove(&inhibitor_id)
            .ok_or(WireError::InvalidObjectId(inhibitor_id.0))?;
        if let Some(count) = self.surface_counts.get_mut(&state.surface_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.surface_counts.remove(&state.surface_id);
            }
        }
        self.inhibited = !self.inhibitors.is_empty();
        Ok(())
    }

    pub fn surface_destroyed(&mut self, surface_id: WaylandObjectId) -> Vec<WaylandObjectId> {
        let mut removed = Vec::new();
        self.inhibitors.retain(|id, state| {
            let keep = state.surface_id != surface_id;
            if !keep {
                removed.push(*id);
            }
            keep
        });
        self.surface_counts.remove(&surface_id);
        self.inhibited = !self.inhibitors.is_empty();
        removed
    }

    pub fn is_inhibited(&self) -> bool {
        self.inhibited
    }
}
