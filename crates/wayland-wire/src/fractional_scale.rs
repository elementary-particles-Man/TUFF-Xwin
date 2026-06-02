use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

pub struct FractionalScaleManager {
    pub scales: HashMap<WaylandObjectId, FractionalScaleState>,
}

pub struct FractionalScaleState {
    pub surface_id: WaylandObjectId,
    pub preferred_scale: u32, // scale * 120
}

impl FractionalScaleManager {
    pub fn new() -> Self {
        Self { scales: HashMap::new() }
    }

    pub fn get_fractional_scale(
        &mut self,
        id: WaylandObjectId,
        surface_id: WaylandObjectId,
    ) -> Result<()> {
        if self.scales.values().any(|s| s.surface_id == surface_id) {
            return Err(WireError::ProtocolError(
                "fractional_scale already exists for this surface".into(),
            ));
        }
        self.scales.insert(
            id,
            FractionalScaleState {
                surface_id,
                preferred_scale: 120, // Default 1.0x
            },
        );
        Ok(())
    }

    pub fn destroy(&mut self, id: WaylandObjectId) {
        self.scales.remove(&id);
    }
}
