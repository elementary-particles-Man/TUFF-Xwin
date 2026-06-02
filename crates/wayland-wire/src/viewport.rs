use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ViewportData {
    pub source: Option<RectFixed>,
    pub destination: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Copy)]
pub struct RectFixed {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub struct ViewportManager {
    pub viewports: HashMap<WaylandObjectId, ViewportState>,
}

pub struct ViewportState {
    pub surface_id: WaylandObjectId,
    pub pending: ViewportData,
    pub current: ViewportData,
}

impl ViewportManager {
    pub fn new() -> Self {
        Self { viewports: HashMap::new() }
    }

    pub fn get_viewport(&mut self, id: WaylandObjectId, surface_id: WaylandObjectId) -> Result<()> {
        if self.viewports.values().any(|v| v.surface_id == surface_id) {
            return Err(WireError::ProtocolError(
                "viewport already exists for this surface".into(),
            ));
        }
        self.viewports.insert(
            id,
            ViewportState {
                surface_id,
                pending: ViewportData::default(),
                current: ViewportData::default(),
            },
        );
        Ok(())
    }

    pub fn set_source(
        &mut self,
        id: WaylandObjectId,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<()> {
        let v = self.viewports.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        if w <= 0 || h <= 0 {
            return Err(WireError::ProtocolError("invalid source size".into()));
        }
        v.pending.source = Some(RectFixed { x, y, width: w, height: h });
        Ok(())
    }

    pub fn set_destination(&mut self, id: WaylandObjectId, w: i32, h: i32) -> Result<()> {
        let v = self.viewports.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        if w <= 0 || h <= 0 {
            return Err(WireError::ProtocolError("invalid destination size".into()));
        }
        v.pending.destination = Some((w, h));
        Ok(())
    }

    pub fn commit(&mut self, surface_id: WaylandObjectId) {
        if let Some(v) = self.viewports.values_mut().find(|v| v.surface_id == surface_id) {
            v.current = v.pending.clone();
        }
    }

    pub fn destroy(&mut self, id: WaylandObjectId) {
        self.viewports.remove(&id);
    }
}
