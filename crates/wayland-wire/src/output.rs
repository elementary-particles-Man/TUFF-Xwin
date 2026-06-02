use crate::WaylandObjectId;
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
}
