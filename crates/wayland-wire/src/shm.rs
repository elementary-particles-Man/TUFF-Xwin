use crate::{FakeFd, Result, WaylandObjectId, WireError};
use std::collections::HashMap;

pub struct ShmPool {
    pub id: WaylandObjectId,
    pub fd: FakeFd,
    pub size: u32,
    pub data: Vec<u8>, // Fake backing memory
}

pub struct ShmBuffer {
    pub id: WaylandObjectId,
    pub pool_id: WaylandObjectId,
    pub offset: i32,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub format: u32,
}

pub struct ShmManager {
    pub pools: HashMap<WaylandObjectId, ShmPool>,
    pub buffers: HashMap<WaylandObjectId, ShmBuffer>,
}

impl ShmManager {
    pub fn new() -> Self {
        Self { pools: HashMap::new(), buffers: HashMap::new() }
    }

    pub fn create_pool(&mut self, id: WaylandObjectId, fd: FakeFd, size: u32) {
        self.pools.insert(id, ShmPool { id, fd, size, data: vec![0u8; size as usize] });
    }

    pub fn create_buffer(
        &mut self,
        id: WaylandObjectId,
        pool_id: WaylandObjectId,
        offset: i32,
        width: i32,
        height: i32,
        stride: i32,
        format: u32,
    ) -> Result<()> {
        let pool = self.pools.get(&pool_id).ok_or(WireError::InvalidObjectId(pool_id.0))?;

        if offset < 0 || stride < 0 || height < 0 {
            return Err(WireError::InvalidSize(0));
        }

        let total_size = (offset as u32) + (stride as u32) * (height as u32);
        if total_size > pool.size {
            return Err(WireError::InvalidSize(total_size));
        }

        self.buffers.insert(id, ShmBuffer { id, pool_id, offset, width, height, stride, format });

        Ok(())
    }
}
