use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

pub enum ShmPoolStorage {
    FakeMemory(Vec<u8>),
    ReceivedFd(crate::WireOwnedFd),
}

pub struct ShmPool {
    pub id: WaylandObjectId,
    pub storage: ShmPoolStorage,
    pub size: u32,
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

    pub fn create_pool_from_fake(&mut self, id: WaylandObjectId, size: u32) {
        self.pools.insert(
            id,
            ShmPool { id, storage: ShmPoolStorage::FakeMemory(vec![0u8; size as usize]), size },
        );
    }

    pub fn create_pool_from_fd(&mut self, id: WaylandObjectId, fd: crate::WireOwnedFd, size: u32) {
        self.pools.insert(id, ShmPool { id, storage: ShmPoolStorage::ReceivedFd(fd), size });
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

        if offset < 0 || stride < 0 || height < 0 || width < 0 {
            return Err(WireError::InvalidSize(0));
        }

        // Basic validation for common formats (Argb8888 = 0, Xrgb8888 = 1)
        if (format == 0 || format == 1) && stride < width * 4 {
            return Err(WireError::ProtocolError("stride too small for format".into()));
        }

        let total_size = (offset as u32) + (stride as u32) * (height as u32);
        if total_size > pool.size {
            return Err(WireError::InvalidSize(total_size));
        }

        self.buffers.insert(id, ShmBuffer { id, pool_id, offset, width, height, stride, format });

        Ok(())
    }
}
