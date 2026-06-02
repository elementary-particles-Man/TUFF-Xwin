use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct TextInputData {
    pub surrounding_text: Option<String>,
    pub cursor: i32,
    pub anchor: i32,
    pub hint: u32,
    pub purpose: u32,
    pub cause: u32,
    pub cursor_rectangle: (i32, i32, i32, i32), // x, y, w, h
}

#[derive(Debug, Clone)]
pub struct TextInputState {
    pub seat_id: WaylandObjectId,
    pub focused_surface_id: Option<WaylandObjectId>,
    pub enabled: bool,
    pub pending: TextInputData,
    pub current: TextInputData,
}

pub struct TextInputManager {
    pub inputs: HashMap<WaylandObjectId, TextInputState>,
}

impl TextInputManager {
    pub fn new() -> Self {
        Self { inputs: HashMap::new() }
    }

    pub fn create_text_input(&mut self, id: WaylandObjectId, seat_id: WaylandObjectId) {
        self.inputs.insert(
            id,
            TextInputState {
                seat_id,
                focused_surface_id: None,
                enabled: false,
                pending: TextInputData::default(),
                current: TextInputData::default(),
            },
        );
    }

    pub fn commit(&mut self, id: WaylandObjectId) -> Result<()> {
        let state = self.inputs.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.current = state.pending.clone();
        Ok(())
    }

    pub fn enable(&mut self, id: WaylandObjectId) -> Result<()> {
        let state = self.inputs.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.enabled = true;
        Ok(())
    }

    pub fn disable(&mut self, id: WaylandObjectId) -> Result<()> {
        let state = self.inputs.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.enabled = false;
        Ok(())
    }
}
