use crate::{WaylandObjectId, Result, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct XdgSurfaceState {
    pub wl_surface_id: WaylandObjectId,
    pub configure_serial: u32,
    pub acked_serial: u32,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

pub struct XdgShellManager {
    pub surfaces: HashMap<WaylandObjectId, XdgSurfaceState>,
    next_serial: u32,
}

impl XdgShellManager {
    pub fn new() -> Self {
        Self {
            surfaces: HashMap::new(),
            next_serial: 1,
        }
    }

    pub fn create_xdg_surface(&mut self, id: WaylandObjectId, wl_surface_id: WaylandObjectId) {
        self.surfaces.insert(id, XdgSurfaceState::new(wl_surface_id));
    }
}

impl XdgSurfaceState {
    pub fn new(wl_surface_id: WaylandObjectId) -> Self {
        Self {
            wl_surface_id,
            configure_serial: 0,
            acked_serial: 0,
            title: None,
            app_id: None,
        }
    }
}

impl XdgShellManager {
    pub fn get_next_serial(&mut self) -> u32 {
        let s = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1);
        s
    }

    pub fn ack_configure(&mut self, id: WaylandObjectId, serial: u32) -> Result<()> {
        let surface = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        if serial > surface.configure_serial && surface.configure_serial != 0 {
             // Basic validation: can't ack serial we haven't sent (though wrapping makes this tricky)
             // For parity P6 we just record it.
        }
        surface.acked_serial = serial;
        Ok(())
    }
}
