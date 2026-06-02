use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct InputMethodState {
    pub seat_id: WaylandObjectId,
    pub active: bool,
    pub done_serial: u32,
}

#[derive(Debug, Clone)]
pub struct InputPopupSurfaceState {
    pub surface_id: WaylandObjectId,
    pub parent_surface_id: WaylandObjectId,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub struct InputMethodManager {
    pub methods: HashMap<WaylandObjectId, InputMethodState>,
    pub popups: HashMap<WaylandObjectId, InputPopupSurfaceState>,
}

impl InputMethodManager {
    pub fn new() -> Self {
        Self { methods: HashMap::new(), popups: HashMap::new() }
    }

    pub fn get_input_method(
        &mut self,
        id: WaylandObjectId,
        seat_id: WaylandObjectId,
    ) -> Result<()> {
        if self.methods.values().any(|m| m.seat_id == seat_id) {
            return Err(WireError::ProtocolError(
                "input method already exists for this seat".into(),
            ));
        }
        self.methods.insert(id, InputMethodState { seat_id, active: false, done_serial: 0 });
        Ok(())
    }

    pub fn create_popup(
        &mut self,
        id: WaylandObjectId,
        surface_id: WaylandObjectId,
        parent_id: WaylandObjectId,
    ) {
        self.popups.insert(
            id,
            InputPopupSurfaceState {
                surface_id,
                parent_surface_id: parent_id,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
        );
    }
}
