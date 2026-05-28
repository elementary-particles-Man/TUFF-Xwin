use crate::WaylandObjectId;
use std::collections::HashMap;

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

pub struct SurfaceManager {
    pub surfaces: HashMap<WaylandObjectId, SurfaceInstance>,
    pub regions: HashMap<WaylandObjectId, RegionState>,
}

pub struct SurfaceInstance {
    pub pending: SurfaceState,
    pub current: SurfaceState,
    pub callbacks: Vec<WaylandObjectId>,
}

impl SurfaceManager {
    pub fn new() -> Self {
        Self { surfaces: HashMap::new(), regions: HashMap::new() }
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
}
