use crate::{Result, WaylandMessage, WaylandObjectId, WaylandOpcode, WireError};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerConstraintKind {
    Locked,
    Confined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerConstraintLifetime {
    OneShot,
    Persistent,
}

impl PointerConstraintLifetime {
    pub fn from_raw(value: u32) -> Result<Self> {
        match value {
            0 => Ok(Self::OneShot),
            1 => Ok(Self::Persistent),
            _ => Err(WireError::ProtocolError(format!("invalid constraint lifetime: {}", value))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PointerConstraintState {
    pub kind: PointerConstraintKind,
    pub surface_id: WaylandObjectId,
    pub pointer_id: WaylandObjectId,
    pub region_id: Option<WaylandObjectId>,
    pub lifetime: PointerConstraintLifetime,
    pub cursor_hint: Option<(i32, i32)>,
}

#[derive(Debug, Default)]
pub struct PointerConstraintsState {
    pub locked: HashMap<WaylandObjectId, PointerConstraintState>,
    pub confined: HashMap<WaylandObjectId, PointerConstraintState>,
    pub active_pairs: HashSet<(WaylandObjectId, WaylandObjectId)>,
}

impl PointerConstraintsState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_locked(
        &mut self,
        id: WaylandObjectId,
        surface_id: WaylandObjectId,
        pointer_id: WaylandObjectId,
        region_id: Option<WaylandObjectId>,
        lifetime: PointerConstraintLifetime,
    ) -> Result<WaylandMessage> {
        self.validate_pair(surface_id, pointer_id)?;
        self.active_pairs.insert((surface_id, pointer_id));
        self.locked.insert(
            id,
            PointerConstraintState {
                kind: PointerConstraintKind::Locked,
                surface_id,
                pointer_id,
                region_id,
                lifetime,
                cursor_hint: None,
            },
        );
        Ok(self.encode_event(id, 0))
    }

    pub fn create_confined(
        &mut self,
        id: WaylandObjectId,
        surface_id: WaylandObjectId,
        pointer_id: WaylandObjectId,
        region_id: Option<WaylandObjectId>,
        lifetime: PointerConstraintLifetime,
    ) -> Result<WaylandMessage> {
        self.validate_pair(surface_id, pointer_id)?;
        self.active_pairs.insert((surface_id, pointer_id));
        self.confined.insert(
            id,
            PointerConstraintState {
                kind: PointerConstraintKind::Confined,
                surface_id,
                pointer_id,
                region_id,
                lifetime,
                cursor_hint: None,
            },
        );
        Ok(self.encode_event(id, 0))
    }

    pub fn set_cursor_position_hint(&mut self, id: WaylandObjectId, x: i32, y: i32) -> Result<()> {
        let state = self.locked.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.cursor_hint = Some((x, y));
        Ok(())
    }

    pub fn set_region(
        &mut self,
        id: WaylandObjectId,
        region: Option<WaylandObjectId>,
    ) -> Result<()> {
        if let Some(state) = self.locked.get_mut(&id) {
            state.region_id = region;
            return Ok(());
        }
        if let Some(state) = self.confined.get_mut(&id) {
            state.region_id = region;
            return Ok(());
        }
        Err(WireError::InvalidObjectId(id.0))
    }

    pub fn destroy_locked(&mut self, id: WaylandObjectId) -> Result<WaylandMessage> {
        let state = self.locked.remove(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        self.active_pairs.remove(&(state.surface_id, state.pointer_id));
        Ok(self.encode_event(id, 1))
    }

    pub fn destroy_confined(&mut self, id: WaylandObjectId) -> Result<WaylandMessage> {
        let state = self.confined.remove(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        self.active_pairs.remove(&(state.surface_id, state.pointer_id));
        Ok(self.encode_event(id, 1))
    }

    pub fn surface_destroyed(&mut self, surface_id: WaylandObjectId) -> Vec<WaylandMessage> {
        let mut events = Vec::new();
        let locked_ids: Vec<_> = self
            .locked
            .iter()
            .filter(|(_, state)| state.surface_id == surface_id)
            .map(|(id, _)| *id)
            .collect();
        for id in locked_ids {
            if let Some(state) = self.locked.remove(&id) {
                self.active_pairs.remove(&(state.surface_id, state.pointer_id));
                events.push(self.encode_event(id, 1));
            }
        }
        let confined_ids: Vec<_> = self
            .confined
            .iter()
            .filter(|(_, state)| state.surface_id == surface_id)
            .map(|(id, _)| *id)
            .collect();
        for id in confined_ids {
            if let Some(state) = self.confined.remove(&id) {
                self.active_pairs.remove(&(state.surface_id, state.pointer_id));
                events.push(self.encode_event(id, 1));
            }
        }
        events
    }

    pub fn pointer_released(&mut self, pointer_id: WaylandObjectId) -> Vec<WaylandMessage> {
        let mut events = Vec::new();
        let locked_ids: Vec<_> = self
            .locked
            .iter()
            .filter(|(_, state)| state.pointer_id == pointer_id)
            .map(|(id, _)| *id)
            .collect();
        for id in locked_ids {
            if let Some(state) = self.locked.remove(&id) {
                self.active_pairs.remove(&(state.surface_id, state.pointer_id));
                events.push(self.encode_event(id, 1));
            }
        }
        let confined_ids: Vec<_> = self
            .confined
            .iter()
            .filter(|(_, state)| state.pointer_id == pointer_id)
            .map(|(id, _)| *id)
            .collect();
        for id in confined_ids {
            if let Some(state) = self.confined.remove(&id) {
                self.active_pairs.remove(&(state.surface_id, state.pointer_id));
                events.push(self.encode_event(id, 1));
            }
        }
        events
    }

    fn validate_pair(
        &self,
        surface_id: WaylandObjectId,
        pointer_id: WaylandObjectId,
    ) -> Result<()> {
        if self.active_pairs.contains(&(surface_id, pointer_id)) {
            return Err(WireError::ProtocolError(
                "duplicate pointer constraint for surface/pointer pair".into(),
            ));
        }
        Ok(())
    }

    fn encode_event(&self, object_id: WaylandObjectId, opcode: u16) -> WaylandMessage {
        crate::codec::encode_event(
            object_id,
            WaylandOpcode(opcode),
            &[],
            &crate::registry::WireObjectRegistry::default(),
        )
        .expect("encode pointer constraint event")
    }
}
