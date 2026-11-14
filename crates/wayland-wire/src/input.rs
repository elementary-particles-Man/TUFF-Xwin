use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Default)]
pub struct SeatManager {
    pub seats: HashMap<WaylandObjectId, SeatState>,
    pub pointers: HashMap<WaylandObjectId, PointerState>,
    pub keyboards: HashMap<WaylandObjectId, KeyboardState>,
    pub touches: HashMap<WaylandObjectId, TouchState>,
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

#[derive(Debug, Clone)]
pub struct TouchState {
    pub seat_id: WaylandObjectId,
}

impl SeatManager {
    pub fn new() -> Self {
        Self {
            seats: HashMap::new(),
            pointers: HashMap::new(),
            keyboards: HashMap::new(),
            touches: HashMap::new(),
        }
    }

    pub fn create_seat(&mut self, id: WaylandObjectId, name: &str) {
        self.seats.insert(
            id,
            SeatState {
                name: name.into(),
                capabilities: 3, // pointer | keyboard; no physical touch backend
            },
        );
    }

    pub fn get_pointer(&mut self, seat_id: WaylandObjectId, new_id: WaylandObjectId) -> Result<()> {
        if !self.seats.contains_key(&seat_id) {
            return Err(WireError::InvalidObjectId(seat_id.0));
        }
        if self.pointers.contains_key(&new_id)
            || self.keyboards.contains_key(&new_id)
            || self.touches.contains_key(&new_id)
        {
            return Err(WireError::InvalidObjectId(new_id.0));
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
        if self.pointers.contains_key(&new_id)
            || self.keyboards.contains_key(&new_id)
            || self.touches.contains_key(&new_id)
        {
            return Err(WireError::InvalidObjectId(new_id.0));
        }
        self.keyboards.insert(new_id, KeyboardState { seat_id, focus_surface_id: None });
        Ok(())
    }

    pub fn get_touch(&mut self, seat_id: WaylandObjectId, new_id: WaylandObjectId) -> Result<()> {
        if !self.seats.contains_key(&seat_id) {
            return Err(WireError::InvalidObjectId(seat_id.0));
        }
        if self.pointers.contains_key(&new_id)
            || self.keyboards.contains_key(&new_id)
            || self.touches.contains_key(&new_id)
        {
            return Err(WireError::InvalidObjectId(new_id.0));
        }
        self.touches.insert(new_id, TouchState { seat_id });
        Ok(())
    }

    pub fn release_pointer(&mut self, pointer_id: WaylandObjectId) -> Result<()> {
        if self.pointers.remove(&pointer_id).is_some() {
            Ok(())
        } else {
            Err(WireError::InvalidObjectId(pointer_id.0))
        }
    }

    pub fn release_keyboard(&mut self, keyboard_id: WaylandObjectId) -> Result<()> {
        self.keyboards
            .remove(&keyboard_id)
            .map(|_| ())
            .ok_or(WireError::InvalidObjectId(keyboard_id.0))
    }

    pub fn release_touch(&mut self, touch_id: WaylandObjectId) -> Result<()> {
        self.touches.remove(&touch_id).map(|_| ()).ok_or(WireError::InvalidObjectId(touch_id.0))
    }

    pub fn release_seat(&mut self, seat_id: WaylandObjectId) -> Result<()> {
        self.seats.remove(&seat_id).map(|_| ()).ok_or(WireError::InvalidObjectId(seat_id.0))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_capabilities_do_not_advertise_unimplemented_touch() {
        let mut manager = SeatManager::new();
        manager.create_seat(WaylandObjectId(7), "seat0");
        assert_eq!(manager.seats[&WaylandObjectId(7)].capabilities, 3);
    }

    #[test]
    fn touch_registration_release_and_id_reuse_are_explicit() {
        let mut manager = SeatManager::new();
        let seat = WaylandObjectId(7);
        let touch = WaylandObjectId(8);
        manager.create_seat(seat, "seat0");
        manager.get_touch(seat, touch).unwrap();
        assert_eq!(manager.touches[&touch].seat_id, seat);
        assert!(manager.get_touch(seat, touch).is_err());
        manager.release_touch(touch).unwrap();
        assert!(manager.release_touch(touch).is_err());
        manager.get_touch(seat, touch).unwrap();
    }

    #[test]
    fn invalid_seat_or_shared_object_id_has_no_touch_state_change() {
        let mut manager = SeatManager::new();
        let touch = WaylandObjectId(8);
        assert!(manager.get_touch(WaylandObjectId(7), touch).is_err());
        manager.create_seat(WaylandObjectId(7), "seat0");
        manager.get_pointer(WaylandObjectId(7), touch).unwrap();
        assert!(manager.get_touch(WaylandObjectId(7), touch).is_err());
        assert!(manager.touches.is_empty());
    }
}
