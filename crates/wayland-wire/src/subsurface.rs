use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

pub struct SubcompositorManager {
    pub subsurfaces: HashMap<WaylandObjectId, SubsurfaceState>,
}

#[derive(Debug, Clone)]
pub struct SubsurfaceState {
    pub surface_id: WaylandObjectId,
    pub parent_id: WaylandObjectId,
    pub x: i32,
    pub y: i32,
    pub sync: bool,
}

impl SubcompositorManager {
    pub fn new() -> Self {
        Self { subsurfaces: HashMap::new() }
    }

    pub fn get_subsurface(
        &mut self,
        id: WaylandObjectId,
        surface_id: WaylandObjectId,
        parent_id: WaylandObjectId,
    ) -> Result<()> {
        // Validation: surface cannot be its own parent
        if surface_id == parent_id {
            return Err(WireError::ProtocolError("surface cannot be its own parent".into()));
        }

        // Validation: surface cannot already be a subsurface
        if self.subsurfaces.values().any(|s| s.surface_id == surface_id) {
            return Err(WireError::ProtocolError("surface is already a subsurface".into()));
        }

        self.subsurfaces.insert(
            id,
            SubsurfaceState {
                surface_id,
                parent_id,
                x: 0,
                y: 0,
                sync: true, // Default is sync
            },
        );
        Ok(())
    }

    pub fn set_position(&mut self, id: WaylandObjectId, x: i32, y: i32) -> Result<()> {
        let state = self.subsurfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.x = x;
        state.y = y;
        Ok(())
    }

    pub fn set_sync(&mut self, id: WaylandObjectId, sync: bool) -> Result<()> {
        let state = self.subsurfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.sync = sync;
        Ok(())
    }

    pub fn destroy(&mut self, id: WaylandObjectId) -> Result<()> {
        self.subsurfaces.remove(&id);
        Ok(())
    }
}
