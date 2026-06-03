use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

pub struct SeatManager {
    pub seats: HashMap<WaylandObjectId, SeatState>,
    pub pointers: HashMap<WaylandObjectId, PointerState>,
    pub keyboards: HashMap<WaylandObjectId, KeyboardState>,
}

#[derive(Debug, Clone)]
pub struct SeatState {
    pub name: String,
    pub capabilities: u32, // 1=Pointer, 2=Keyboard, 4=Touch
}

#[derive(Debug, Clone)]
pub struct PointerState {
    pub seat_id: WaylandObjectId,
    pub focus_surface_id: Option<WaylandObjectId>,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone)]
pub struct KeyboardState {
    pub seat_id: WaylandObjectId,
    pub focus_surface_id: Option<WaylandObjectId>,
}

impl SeatManager {
    pub fn new() -> Self {
        Self { seats: HashMap::new(), pointers: HashMap::new(), keyboards: HashMap::new() }
    }

    pub fn create_seat(&mut self, id: WaylandObjectId, name: &str) {
        self.seats.insert(
            id,
            SeatState {
                name: name.into(),
                capabilities: 7, // All by default for headless parity
            },
        );
    }

    pub fn get_pointer(&mut self, seat_id: WaylandObjectId, new_id: WaylandObjectId) -> Result<()> {
        if !self.seats.contains_key(&seat_id) {
            return Err(WireError::InvalidObjectId(seat_id.0));
        }
        self.pointers
            .insert(new_id, PointerState { seat_id, focus_surface_id: None, x: 0.0, y: 0.0 });
        Ok(())
    }

    pub fn get_keyboard(
        &mut self,
        seat_id: WaylandObjectId,
        new_id: WaylandObjectId,
    ) -> Result<()> {
        if !self.seats.contains_key(&seat_id) {
            return Err(WireError::InvalidObjectId(seat_id.0));
        }
        self.keyboards.insert(new_id, KeyboardState { seat_id, focus_surface_id: None });
        Ok(())
    }

    pub fn release_pointer(&mut self, pointer_id: WaylandObjectId) -> Result<()> {
        if self.pointers.remove(&pointer_id).is_some() {
            Ok(())
        } else {
            Err(WireError::InvalidObjectId(pointer_id.0))
        }
    }

    pub fn set_pointer_focus(
        &mut self,
        pointer_id: WaylandObjectId,
        surface_id: Option<WaylandObjectId>,
        x: f64,
        y: f64,
    ) -> Result<()> {
        let pointer =
            self.pointers.get_mut(&pointer_id).ok_or(WireError::InvalidObjectId(pointer_id.0))?;
        pointer.focus_surface_id = surface_id;
        pointer.x = x;
        pointer.y = y;
        Ok(())
    }
}

pub enum FakeInputEvent {
    PointerEnter { id: WaylandObjectId, surface_id: WaylandObjectId, x: f64, y: f64 },
    PointerMotion { id: WaylandObjectId, x: f64, y: f64 },
    PointerButton { id: WaylandObjectId, button: u32, state: u32 },
    KeyboardEnter { id: WaylandObjectId, surface_id: WaylandObjectId },
    KeyboardKey { id: WaylandObjectId, key: u32, state: u32 },
}
