use crate::{Result, WaylandMessage, WaylandObjectId, WaylandOpcode, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct RelativePointerBinding {
    pub pointer_id: WaylandObjectId,
}

#[derive(Debug, Default)]
pub struct RelativePointerState {
    pub bindings: HashMap<WaylandObjectId, RelativePointerBinding>,
    pub bindings_by_pointer: HashMap<WaylandObjectId, Vec<WaylandObjectId>>,
}

impl RelativePointerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_relative_pointer(
        &mut self,
        relative_id: WaylandObjectId,
        pointer_id: WaylandObjectId,
    ) -> Result<()> {
        if self.bindings.contains_key(&relative_id) {
            return Err(WireError::InvalidObjectId(relative_id.0));
        }
        self.bindings.insert(relative_id, RelativePointerBinding { pointer_id });
        self.bindings_by_pointer.entry(pointer_id).or_default().push(relative_id);
        Ok(())
    }

    pub fn destroy(&mut self, relative_id: WaylandObjectId) -> Result<()> {
        let binding =
            self.bindings.remove(&relative_id).ok_or(WireError::InvalidObjectId(relative_id.0))?;
        if let Some(list) = self.bindings_by_pointer.get_mut(&binding.pointer_id) {
            list.retain(|id| *id != relative_id);
            if list.is_empty() {
                self.bindings_by_pointer.remove(&binding.pointer_id);
            }
        }
        Ok(())
    }

    pub fn pointer_released(&mut self, pointer_id: WaylandObjectId) -> Vec<WaylandObjectId> {
        let removed = self.bindings_by_pointer.remove(&pointer_id).unwrap_or_default();
        for id in &removed {
            self.bindings.remove(id);
        }
        removed
    }

    pub fn inject_relative_motion(
        &self,
        pointer_id: WaylandObjectId,
        time_usec: u64,
        dx: f64,
        dy: f64,
        dx_unaccel: f64,
        dy_unaccel: f64,
    ) -> Vec<WaylandMessage> {
        let mut events = Vec::new();
        if let Some(ids) = self.bindings_by_pointer.get(&pointer_id) {
            let hi = (time_usec >> 32) as u32;
            let lo = (time_usec & 0xffff_ffff) as u32;
            for relative_id in ids {
                let args = vec![
                    crate::WireArg::Uint(hi),
                    crate::WireArg::Uint(lo),
                    crate::WireArg::Fixed((dx * 256.0) as i32),
                    crate::WireArg::Fixed((dy * 256.0) as i32),
                    crate::WireArg::Fixed((dx_unaccel * 256.0) as i32),
                    crate::WireArg::Fixed((dy_unaccel * 256.0) as i32),
                ];
                events.push(
                    crate::codec::encode_event(
                        *relative_id,
                        WaylandOpcode(0),
                        &args,
                        &crate::registry::WireObjectRegistry::default(),
                    )
                    .expect("encode relative pointer event"),
                );
            }
        }
        events
    }
}
