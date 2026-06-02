use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DataSource {
    pub mime_types: Vec<String>,
    pub dnd_actions: u32,
    pub is_destroyed: bool,
}

#[derive(Debug, Clone)]
pub struct DataOffer {
    pub source_id: Option<WaylandObjectId>,
    pub mime_types: Vec<String>,
    pub dnd_actions: u32,
    pub preferred_action: u32,
    pub is_destroyed: bool,
}

#[derive(Debug, Clone)]
pub struct DataDevice {
    pub seat_id: WaylandObjectId,
}

pub struct ActiveDrag {
    pub source_id: Option<WaylandObjectId>,
    pub origin_surface_id: WaylandObjectId,
    pub icon_surface_id: Option<WaylandObjectId>,
    pub target_surface_id: Option<WaylandObjectId>,
    pub offer_id: Option<WaylandObjectId>,
}

pub struct DataDeviceManager {
    pub sources: HashMap<WaylandObjectId, DataSource>,
    pub offers: HashMap<WaylandObjectId, DataOffer>,
    pub devices: HashMap<WaylandObjectId, DataDevice>,
    pub seat_selections: HashMap<WaylandObjectId, Option<WaylandObjectId>>, // seat -> source_id
    pub active_drags: HashMap<WaylandObjectId, ActiveDrag>,                 // seat -> drag
}

impl DataDeviceManager {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            offers: HashMap::new(),
            devices: HashMap::new(),
            seat_selections: HashMap::new(),
            active_drags: HashMap::new(),
        }
    }

    pub fn create_data_source(&mut self, id: WaylandObjectId) {
        self.sources
            .insert(id, DataSource { mime_types: Vec::new(), dnd_actions: 0, is_destroyed: false });
    }

    pub fn get_data_device(&mut self, id: WaylandObjectId, seat_id: WaylandObjectId) {
        self.devices.insert(id, DataDevice { seat_id });
    }

    pub fn start_drag(
        &mut self,
        seat_id: WaylandObjectId,
        source_id: Option<WaylandObjectId>,
        origin_id: WaylandObjectId,
        icon_id: Option<WaylandObjectId>,
    ) {
        self.active_drags.insert(
            seat_id,
            ActiveDrag {
                source_id,
                origin_surface_id: origin_id,
                icon_surface_id: icon_id,
                target_surface_id: None,
                offer_id: None,
            },
        );
    }

    pub fn end_drag(&mut self, seat_id: WaylandObjectId) {
        self.active_drags.remove(&seat_id);
    }
}
