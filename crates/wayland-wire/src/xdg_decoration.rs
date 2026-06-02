use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationMode {
    ClientSide = 1,
    ServerSide = 2,
}

pub struct XdgDecorationManager {
    pub decorations: HashMap<WaylandObjectId, XdgToplevelDecorationState>,
}

pub struct XdgToplevelDecorationState {
    pub toplevel_id: WaylandObjectId,
    pub mode: Option<DecorationMode>,
}

impl XdgDecorationManager {
    pub fn new() -> Self {
        Self { decorations: HashMap::new() }
    }

    pub fn get_toplevel_decoration(
        &mut self,
        id: WaylandObjectId,
        toplevel_id: WaylandObjectId,
    ) -> Result<()> {
        if self.decorations.values().any(|d| d.toplevel_id == toplevel_id) {
            return Err(WireError::ProtocolError(
                "decoration already exists for this toplevel".into(),
            ));
        }
        self.decorations.insert(id, XdgToplevelDecorationState { toplevel_id, mode: None });
        Ok(())
    }

    pub fn set_mode(&mut self, id: WaylandObjectId, mode: u32) -> Result<DecorationMode> {
        let d = self.decorations.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        let m = match mode {
            1 => DecorationMode::ClientSide,
            2 => DecorationMode::ServerSide,
            _ => return Err(WireError::ProtocolError("invalid decoration mode".into())),
        };
        d.mode = Some(m);
        Ok(m)
    }

    pub fn destroy(&mut self, id: WaylandObjectId) {
        self.decorations.remove(&id);
    }
}
