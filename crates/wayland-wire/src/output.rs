use crate::{surface::Rect, WaylandObjectId};
use std::collections::HashMap;

pub struct OutputManager {
    pub outputs: HashMap<WaylandObjectId, OutputState>,
}

#[derive(Debug, Clone)]
pub struct OutputState {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: i32,
    pub fractional_scale: u32, // scale * 120
    pub refresh_nsec: u32,
}

impl OutputManager {
    pub fn new() -> Self {
        Self { outputs: HashMap::new() }
    }

    pub fn create_output(&mut self, id: WaylandObjectId, name: &str) {
        self.outputs.insert(
            id,
            OutputState {
                name: name.into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1,
                fractional_scale: 120,
                refresh_nsec: 16666666, // 60Hz
            },
        );
    }

    pub fn set_mode(&mut self, id: WaylandObjectId, width: i32, height: i32, refresh_nsec: u32) {
        if let Some(output) = self.outputs.get_mut(&id) {
            output.width = width;
            output.height = height;
            output.refresh_nsec = refresh_nsec;
        }
    }

    pub fn geometry_rect(&self, id: WaylandObjectId) -> Option<Rect> {
        self.outputs.get(&id).map(|output| Rect {
            x: output.x,
            y: output.y,
            width: output.width.max(0) as u32,
            height: output.height.max(0) as u32,
        })
    }
}
