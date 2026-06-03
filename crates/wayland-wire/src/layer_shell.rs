use crate::{
    output::OutputManager,
    surface::{Rect, SurfaceManager, SurfaceRoleKind},
    Result, WaylandMessage, WaylandObjectId, WaylandOpcode, WireError,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerMargins {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Default for LayerMargins {
    fn default() -> Self {
        Self { top: 0, right: 0, bottom: 0, left: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub exclusive_zone: i32,
}

#[derive(Debug, Clone)]
pub struct LayerSurfaceState {
    pub wl_surface_id: WaylandObjectId,
    pub output_id: Option<WaylandObjectId>,
    pub layer: u32,
    pub namespace: String,
    pub size: Option<(u32, u32)>,
    pub anchor: u32,
    pub exclusive_zone: i32,
    pub margins: LayerMargins,
    pub keyboard_interactivity: u32,
    pub last_configure_serial: u32,
    pub acked_serial: u32,
    pub popup_id: Option<WaylandObjectId>,
    pub popup_positioner: Option<WaylandObjectId>,
    pub placement: LayerPlacement,
}

#[derive(Debug, Default)]
pub struct LayerShellState {
    pub surfaces: HashMap<WaylandObjectId, LayerSurfaceState>,
    pub surface_bindings: HashMap<WaylandObjectId, WaylandObjectId>,
    next_serial: u32,
}

impl LayerShellState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_layer_surface(
        &mut self,
        id: WaylandObjectId,
        wl_surface_id: WaylandObjectId,
        output_id: Option<WaylandObjectId>,
        layer: u32,
        namespace: String,
        surfaces: &mut SurfaceManager,
        outputs: &OutputManager,
    ) -> Result<WaylandMessage> {
        if self.surface_bindings.contains_key(&wl_surface_id) {
            return Err(WireError::ProtocolError(
                "wl_surface already has a layer-shell role".into(),
            ));
        }
        surfaces.claim_role(wl_surface_id, SurfaceRoleKind::LayerSurface)?;

        let placement = self.calculate_placement(
            output_id,
            LayerMargins::default(),
            None,
            0,
            (0, 0),
            0,
            outputs,
        );
        let serial = self.next_configure_serial();
        let state = LayerSurfaceState {
            wl_surface_id,
            output_id,
            layer,
            namespace,
            size: None,
            anchor: 0,
            exclusive_zone: 0,
            margins: LayerMargins::default(),
            keyboard_interactivity: 0,
            last_configure_serial: serial,
            acked_serial: 0,
            popup_id: None,
            popup_positioner: None,
            placement,
        };
        self.surface_bindings.insert(wl_surface_id, id);
        self.surfaces.insert(id, state);
        Ok(self.encode_configure(id, serial))
    }

    pub fn set_size(
        &mut self,
        id: WaylandObjectId,
        width: u32,
        height: u32,
        outputs: &OutputManager,
    ) -> Result<()> {
        let (output_id, margins, anchor, exclusive_zone, origin, size) = {
            let state = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
            if width == 0 && height == 0 {
                return Err(WireError::ProtocolError("invalid layer-shell size".into()));
            }
            state.size = Some((width, height));
            (
                state.output_id,
                state.margins,
                state.anchor,
                state.exclusive_zone,
                (state.placement.x, state.placement.y),
                state.size,
            )
        };
        let placement = self.calculate_placement(
            output_id,
            margins,
            size,
            anchor,
            origin,
            exclusive_zone,
            outputs,
        );
        if let Some(state) = self.surfaces.get_mut(&id) {
            state.placement = placement;
        }
        Ok(())
    }

    pub fn set_anchor(
        &mut self,
        id: WaylandObjectId,
        anchor: u32,
        outputs: &OutputManager,
    ) -> Result<()> {
        if anchor > 0x0f {
            return Err(WireError::ProtocolError("invalid layer-shell anchor".into()));
        }
        let (output_id, margins, size, origin, exclusive_zone) = {
            let state = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
            state.anchor = anchor;
            (
                state.output_id,
                state.margins,
                state.size,
                (state.placement.x, state.placement.y),
                state.exclusive_zone,
            )
        };
        let placement = self.calculate_placement(
            output_id,
            margins,
            size,
            anchor,
            origin,
            exclusive_zone,
            outputs,
        );
        if let Some(state) = self.surfaces.get_mut(&id) {
            state.placement = placement;
        }
        Ok(())
    }

    pub fn set_exclusive_zone(
        &mut self,
        id: WaylandObjectId,
        zone: i32,
        outputs: &OutputManager,
    ) -> Result<()> {
        if zone < -1 {
            return Err(WireError::ProtocolError("invalid exclusive zone".into()));
        }
        let (output_id, margins, size, anchor, origin) = {
            let state = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
            state.exclusive_zone = zone;
            (
                state.output_id,
                state.margins,
                state.size,
                state.anchor,
                (state.placement.x, state.placement.y),
            )
        };
        let placement =
            self.calculate_placement(output_id, margins, size, anchor, origin, zone, outputs);
        if let Some(state) = self.surfaces.get_mut(&id) {
            state.placement = placement;
        }
        Ok(())
    }

    pub fn set_margin(
        &mut self,
        id: WaylandObjectId,
        top: i32,
        right: i32,
        bottom: i32,
        left: i32,
        outputs: &OutputManager,
    ) -> Result<()> {
        let (output_id, size, anchor, exclusive_zone, origin) = {
            let state = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
            state.margins = LayerMargins { top, right, bottom, left };
            (
                state.output_id,
                state.size,
                state.anchor,
                state.exclusive_zone,
                (state.placement.x, state.placement.y),
            )
        };
        let placement = self.calculate_placement(
            output_id,
            LayerMargins { top, right, bottom, left },
            size,
            anchor,
            origin,
            exclusive_zone,
            outputs,
        );
        if let Some(state) = self.surfaces.get_mut(&id) {
            state.placement = placement;
        }
        Ok(())
    }

    pub fn set_keyboard_interactivity(&mut self, id: WaylandObjectId, value: u32) -> Result<()> {
        let state = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.keyboard_interactivity = value;
        Ok(())
    }

    pub fn set_layer(&mut self, id: WaylandObjectId, layer: u32) -> Result<()> {
        if layer > 3 {
            return Err(WireError::ProtocolError("invalid layer-shell layer".into()));
        }
        let state = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        state.layer = layer;
        Ok(())
    }

    pub fn get_popup(
        &mut self,
        id: WaylandObjectId,
        wl_surface_id: WaylandObjectId,
        positioner_id: WaylandObjectId,
    ) -> Result<()> {
        let state = self
            .surfaces
            .get_mut(&wl_surface_id)
            .ok_or(WireError::InvalidObjectId(wl_surface_id.0))?;
        state.popup_id = Some(id);
        state.popup_positioner = Some(positioner_id);
        Ok(())
    }

    pub fn ack_configure(&mut self, id: WaylandObjectId, serial: u32) -> Result<()> {
        let state = self.surfaces.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        if state.last_configure_serial == 0 {
            return Err(WireError::ProtocolError("layer-shell ack before configure".into()));
        }
        if serial != state.last_configure_serial {
            return Err(WireError::ProtocolError(format!(
                "layer-shell ack serial mismatch: expected {}, got {}",
                state.last_configure_serial, serial
            )));
        }
        state.acked_serial = serial;
        Ok(())
    }

    pub fn surface_destroyed(&mut self, wl_surface_id: WaylandObjectId) -> Vec<WaylandMessage> {
        let mut events = Vec::new();
        if let Some(layer_id) = self.surface_bindings.remove(&wl_surface_id) {
            if self.surfaces.remove(&layer_id).is_some() {
                events.push(self.encode_closed(layer_id));
            }
        }
        events
    }

    pub fn destroy(&mut self, id: WaylandObjectId, surfaces: &mut SurfaceManager) -> Result<()> {
        let state = self.surfaces.remove(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        self.surface_bindings.remove(&state.wl_surface_id);
        surfaces.release_role(state.wl_surface_id);
        Ok(())
    }

    fn next_configure_serial(&mut self) -> u32 {
        let serial = self.next_serial.max(1);
        self.next_serial = serial.wrapping_add(1);
        serial
    }

    fn calculate_placement(
        &self,
        output_id: Option<WaylandObjectId>,
        margins: LayerMargins,
        size: Option<(u32, u32)>,
        _anchor: u32,
        origin: (i32, i32),
        exclusive_zone: i32,
        outputs: &OutputManager,
    ) -> LayerPlacement {
        let geometry = output_id.and_then(|id| outputs.geometry_rect(id)).unwrap_or(Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        });
        let width = size.map(|s| s.0).unwrap_or_else(|| {
            geometry.width.saturating_sub((margins.left + margins.right).max(0) as u32)
        });
        let height = size.map(|s| s.1).unwrap_or_else(|| {
            geometry.height.saturating_sub((margins.top + margins.bottom).max(0) as u32)
        });
        LayerPlacement {
            x: geometry.x + margins.left + origin.0,
            y: geometry.y + margins.top + origin.1,
            width,
            height,
            exclusive_zone,
        }
    }

    fn encode_configure(&self, id: WaylandObjectId, serial: u32) -> WaylandMessage {
        crate::codec::encode_event(
            id,
            WaylandOpcode(0),
            &[crate::WireArg::Uint(serial), crate::WireArg::Uint(0), crate::WireArg::Uint(0)],
            &crate::registry::WireObjectRegistry::default(),
        )
        .expect("encode layer configure")
    }

    fn encode_closed(&self, id: WaylandObjectId) -> WaylandMessage {
        crate::codec::encode_event(
            id,
            WaylandOpcode(1),
            &[],
            &crate::registry::WireObjectRegistry::default(),
        )
        .expect("encode layer closed")
    }
}
