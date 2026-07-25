use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Default)]
pub struct XdgOutputManager {
    pub outputs: HashMap<WaylandObjectId, XdgOutputState>,
}

pub struct XdgOutputState {
    pub wl_output_id: WaylandObjectId,
}

impl XdgOutputManager {
    pub fn new() -> Self {
        Self { outputs: HashMap::new() }
    }

    pub fn get_xdg_output(
        &mut self,
        id: WaylandObjectId,
        wl_output_id: WaylandObjectId,
    ) -> Result<()> {
        if self.outputs.values().any(|o| o.wl_output_id == wl_output_id) {
            return Err(WireError::ProtocolError(
                "xdg_output already exists for this wl_output".into(),
            ));
        }
        self.outputs.insert(id, XdgOutputState { wl_output_id });
        Ok(())
    }

    pub fn destroy(&mut self, id: WaylandObjectId) {
        self.outputs.remove(&id);
    }
}
