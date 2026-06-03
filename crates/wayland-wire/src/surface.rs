use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceRoleKind {
    XdgSurface,
    LayerSurface,
    Subsurface,
    Popup,
}

impl SurfaceRoleKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::XdgSurface => "xdg_surface",
            Self::LayerSurface => "layer_surface",
            Self::Subsurface => "wl_subsurface",
            Self::Popup => "xdg_popup",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RegionState {
    pub rects: Vec<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct SurfaceState {
    pub buffer_id: Option<WaylandObjectId>,
    pub offset_x: i32,
    pub offset_y: i32,
    pub damage: Vec<Rect>,
    pub opaque_region: Option<WaylandObjectId>,
    pub input_region: Option<WaylandObjectId>,
}

impl SurfaceState {
    pub fn snapshot_width_hint(&self) -> Option<u32> {
        self.damage.iter().map(|rect| rect.width).max()
    }

    pub fn snapshot_height_hint(&self) -> Option<u32> {
        self.damage.iter().map(|rect| rect.height).max()
    }
}

pub struct SurfaceManager {
    pub surfaces: HashMap<WaylandObjectId, SurfaceInstance>,
    pub regions: HashMap<WaylandObjectId, RegionState>,
    pub roles: HashMap<WaylandObjectId, SurfaceRoleKind>,
}

pub struct SurfaceInstance {
    pub pending: SurfaceState,
    pub current: SurfaceState,
    pub callbacks: Vec<WaylandObjectId>,
}

impl SurfaceManager {
    pub fn new() -> Self {
        Self { surfaces: HashMap::new(), regions: HashMap::new(), roles: HashMap::new() }
    }

    pub fn create_surface(&mut self, id: WaylandObjectId) {
        self.surfaces.insert(
            id,
            SurfaceInstance {
                pending: SurfaceState::default(),
                current: SurfaceState::default(),
                callbacks: Vec::new(),
            },
        );
    }

    pub fn create_region(&mut self, id: WaylandObjectId) {
        self.regions.insert(id, RegionState::default());
    }

    pub fn commit(&mut self, id: WaylandObjectId) {
        if let Some(surface) = self.surfaces.get_mut(&id) {
            surface.current = surface.pending.clone();
            surface.pending.damage.clear();
        }
    }

    pub fn claim_role(&mut self, id: WaylandObjectId, role: SurfaceRoleKind) -> Result<()> {
        if let Some(existing) = self.roles.get(&id) {
            return Err(WireError::ProtocolError(format!(
                "wl_surface {} already has role {}",
                id.0,
                existing.as_str()
            )));
        }

        self.roles.insert(id, role);
        Ok(())
    }

    pub fn release_role(&mut self, id: WaylandObjectId) {
        self.roles.remove(&id);
    }

    pub fn destroy_surface(&mut self, id: WaylandObjectId) {
        self.surfaces.remove(&id);
        self.roles.remove(&id);
    }
}
