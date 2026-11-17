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

const MAX_REGION_RECTS: usize = 4096;

impl Rect {
    fn bounds(self) -> (i64, i64, i64, i64) {
        (
            self.x as i64,
            self.y as i64,
            self.x as i64 + self.width as i64,
            self.y as i64 + self.height as i64,
        )
    }

    fn from_bounds(x1: i64, y1: i64, x2: i64, y2: i64) -> Option<Self> {
        if x2 <= x1
            || y2 <= y1
            || x1 < i32::MIN as i64
            || y1 < i32::MIN as i64
            || x1 > i32::MAX as i64
            || y1 > i32::MAX as i64
            || x2 - x1 > u32::MAX as i64
            || y2 - y1 > u32::MAX as i64
        {
            return None;
        }
        Some(Self { x: x1 as i32, y: y1 as i32, width: (x2 - x1) as u32, height: (y2 - y1) as u32 })
    }

    fn subtract(self, cut: Rect) -> Vec<Self> {
        let (ax1, ay1, ax2, ay2) = self.bounds();
        let (bx1, by1, bx2, by2) = cut.bounds();
        let ix1 = ax1.max(bx1);
        let iy1 = ay1.max(by1);
        let ix2 = ax2.min(bx2);
        let iy2 = ay2.min(by2);
        if ix1 >= ix2 || iy1 >= iy2 {
            return vec![self];
        }
        [
            Self::from_bounds(ax1, ay1, ax2, iy1),
            Self::from_bounds(ax1, iy2, ax2, ay2),
            Self::from_bounds(ax1, iy1, ix1, iy2),
            Self::from_bounds(ix2, iy1, ax2, iy2),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn add_to(rects: &mut Vec<Self>, addition: Self) -> Result<()> {
        let mut pieces = vec![addition];
        for existing in rects.iter().copied() {
            pieces = pieces.into_iter().flat_map(|piece| piece.subtract(existing)).collect();
        }
        if rects
            .len()
            .checked_add(pieces.len())
            .ok_or_else(|| WireError::ProtocolError("region rectangle count overflow".into()))?
            > MAX_REGION_RECTS
        {
            return Err(WireError::ProtocolError("wl_region rectangle budget exhausted".into()));
        }
        rects.extend(pieces);
        Ok(())
    }

    pub fn validate_region_rect(x: i32, y: i32, width: i32, height: i32) -> Result<Self> {
        if width <= 0 || height <= 0 {
            return Err(WireError::ProtocolError(
                "wl_region rectangle dimensions must be positive".into(),
            ));
        }
        Self::from_bounds(x as i64, y as i64, x as i64 + width as i64, y as i64 + height as i64)
            .ok_or_else(|| WireError::ProtocolError("wl_region rectangle is out of range".into()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct SurfaceState {
    pub buffer_id: Option<WaylandObjectId>,
    pub offset_x: i32,
    pub offset_y: i32,
    pub damage: Vec<Rect>,
    pub opaque_region: Option<Vec<Rect>>,
    pub input_region: Option<Vec<Rect>>,
}

impl SurfaceState {
    pub fn snapshot_width_hint(&self) -> Option<u32> {
        self.damage.iter().map(|rect| rect.width).max()
    }

    pub fn snapshot_height_hint(&self) -> Option<u32> {
        self.damage.iter().map(|rect| rect.height).max()
    }
}

#[derive(Default)]
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

    pub fn region_add(&mut self, id: WaylandObjectId, rect: Rect) -> Result<()> {
        self.regions
            .get_mut(&id)
            .ok_or(WireError::InvalidObjectId(id.0))
            .and_then(|region| Rect::add_to(&mut region.rects, rect))
    }

    pub fn region_subtract(&mut self, id: WaylandObjectId, cut: Rect) -> Result<()> {
        let region = self.regions.get_mut(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        region.rects = region.rects.iter().copied().flat_map(|rect| rect.subtract(cut)).collect();
        Ok(())
    }

    pub fn region_snapshot(&self, id: WaylandObjectId) -> Result<Vec<Rect>> {
        self.regions
            .get(&id)
            .map(|region| region.rects.clone())
            .ok_or(WireError::InvalidObjectId(id.0))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect::validate_region_rect(x, y, w, h).unwrap()
    }

    #[test]
    fn add_removes_overlap_and_subtract_splits() {
        let mut manager = SurfaceManager::new();
        let id = WaylandObjectId(7);
        manager.create_region(id);
        manager.region_add(id, r(0, 0, 10, 10)).unwrap();
        manager.region_add(id, r(5, 5, 10, 10)).unwrap();
        assert_eq!(
            manager.regions[&id]
                .rects
                .iter()
                .map(|x| x.width as i64 * x.height as i64)
                .sum::<i64>(),
            175
        );
        manager.region_subtract(id, r(2, 2, 4, 4)).unwrap();
        assert_eq!(
            manager.regions[&id]
                .rects
                .iter()
                .map(|x| x.width as i64 * x.height as i64)
                .sum::<i64>(),
            159
        );
    }

    #[test]
    fn negative_coordinates_and_invalid_dimensions_are_checked() {
        assert!(Rect::validate_region_rect(-10, -20, 5, 6).is_ok());
        assert!(Rect::validate_region_rect(0, 0, 0, 1).is_err());
        assert!(Rect::validate_region_rect(0, 0, -1, 1).is_err());
    }

    #[test]
    fn surface_region_snapshot_survives_region_destroy_and_commit_isolated() {
        let mut manager = SurfaceManager::new();
        let region = WaylandObjectId(2);
        let surface = WaylandObjectId(3);
        manager.create_region(region);
        manager.region_add(region, r(-4, -5, 8, 9)).unwrap();
        manager.create_surface(surface);
        manager.surfaces.get_mut(&surface).unwrap().pending.opaque_region =
            Some(manager.region_snapshot(region).unwrap());
        assert!(manager.surfaces[&surface].current.opaque_region.is_none());
        manager.commit(surface);
        manager.regions.remove(&region);
        assert_eq!(manager.surfaces[&surface].current.opaque_region.as_ref().unwrap()[0].x, -4);
    }
}
