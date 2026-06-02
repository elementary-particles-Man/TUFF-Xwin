use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct XdgSurfaceState {
    pub wl_surface_id: WaylandObjectId,
    pub configure_serial: u32,
    pub acked_serial: u32,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PositionerState {
    pub width: i32,
    pub height: i32,
    pub anchor_rect: (i32, i32, i32, i32), // x, y, w, h
    pub anchor: u32,
    pub gravity: u32,
    pub constraint_adjustment: u32,
    pub offset: (i32, i32),
}

#[derive(Debug, Clone)]
pub struct PopupState {
    pub xdg_surface_id: WaylandObjectId,
    pub parent_id: Option<WaylandObjectId>,
    pub positioner: PositionerState,
}

pub struct XdgShellManager {
    pub surfaces: HashMap<WaylandObjectId, XdgSurfaceState>,
    pub positioners: HashMap<WaylandObjectId, PositionerState>,
    pub popups: HashMap<WaylandObjectId, PopupState>,
    next_serial: u32,
}

impl XdgShellManager {
    pub fn new() -> Self {
        Self {
            surfaces: HashMap::new(),
            positioners: HashMap::new(),
            popups: HashMap::new(),
            next_serial: 1,
        }
    }

    pub fn create_xdg_surface(&mut self, id: WaylandObjectId, wl_surface_id: WaylandObjectId) {
        self.surfaces.insert(id, XdgSurfaceState::new(wl_surface_id));
    }

    pub fn create_positioner(&mut self, id: WaylandObjectId) {
        self.positioners.insert(id, PositionerState::default());
    }

    pub fn create_popup(
        &mut self,
        id: WaylandObjectId,
        xdg_surface_id: WaylandObjectId,
        parent_id: Option<WaylandObjectId>,
        positioner_id: WaylandObjectId,
    ) -> Result<()> {
        let positioner = self
            .positioners
            .get(&positioner_id)
            .ok_or(WireError::InvalidObjectId(positioner_id.0))?
            .clone();

        self.popups.insert(id, PopupState { xdg_surface_id, parent_id, positioner });
        Ok(())
    }
}

impl XdgSurfaceState {
    pub fn new(wl_surface_id: WaylandObjectId) -> Self {
        Self { wl_surface_id, configure_serial: 0, acked_serial: 0, title: None, app_id: None }
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
        if surface.configure_serial == 0 {
            return Err(WireError::ProtocolError("ack_configure before any configure sent".into()));
        }
        if serial > surface.configure_serial {
            return Err(WireError::ProtocolError(format!(
                "ack_configure with invalid serial: {} (last sent: {})",
                serial, surface.configure_serial
            )));
        }
        surface.acked_serial = serial;
        Ok(())
    }
}
