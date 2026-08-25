use crate::{Result, WaylandObjectId, WireError};
use std::collections::HashMap;

pub const MAX_SHM_BUFFER_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_FAKE_SHM_POOL_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_FAKE_SHM_TOTAL_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_SHM_POOLS: usize = 256;
pub const MAX_SHM_BUFFERS: usize = 4096;

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

impl ShmBuffer {
    pub fn byte_len(&self) -> Option<usize> {
        if self.width <= 0 || self.height <= 0 || self.stride <= 0 {
            return None;
        }
        let bytes = (self.stride as usize).checked_mul(self.height as usize)?;
        (bytes <= MAX_SHM_BUFFER_BYTES).then_some(bytes)
    }
}

#[derive(Default)]
pub struct ShmManager {
    pub pools: HashMap<WaylandObjectId, ShmPool>,
    pub buffers: HashMap<WaylandObjectId, ShmBuffer>,
}

impl ShmManager {
    pub fn new() -> Self {
        Self { pools: HashMap::new(), buffers: HashMap::new() }
    }

    fn validate_new_pool(&self, id: WaylandObjectId, size: u32) -> Result<()> {
        if size == 0 {
            return Err(WireError::InvalidSize(0));
        }
        if self.pools.contains_key(&id) {
            return Err(WireError::ProtocolError("duplicate wl_shm pool identity".into()));
        }
        if self.pools.len() >= MAX_SHM_POOLS {
            return Err(WireError::ProtocolError("wl_shm pool budget exhausted".into()));
        }
        Ok(())
    }

    pub fn create_pool_from_fake(&mut self, id: WaylandObjectId, size: u32) -> Result<()> {
        self.validate_new_pool(id, size)?;
        if size as usize > MAX_FAKE_SHM_POOL_BYTES {
            return Err(WireError::ProtocolError(format!(
                "fake wl_shm pool exceeds {} bytes",
                MAX_FAKE_SHM_POOL_BYTES
            )));
        }
        let retained_fake_bytes = self
            .pools
            .iter()
            .filter(|(existing_id, _)| **existing_id != id)
            .try_fold(0usize, |total, (_, pool)| match &pool.storage {
                ShmPoolStorage::FakeMemory(bytes) => total.checked_add(bytes.len()),
                ShmPoolStorage::ReceivedFd(_) => Some(total),
            })
            .ok_or_else(|| {
                WireError::ProtocolError("fake wl_shm byte accounting overflow".into())
            })?;
        let next_fake_bytes = retained_fake_bytes.checked_add(size as usize).ok_or_else(|| {
            WireError::ProtocolError("fake wl_shm byte accounting overflow".into())
        })?;
        if next_fake_bytes > MAX_FAKE_SHM_TOTAL_BYTES {
            return Err(WireError::ProtocolError(format!(
                "aggregate fake wl_shm pools exceed {} bytes",
                MAX_FAKE_SHM_TOTAL_BYTES
            )));
        }
        self.pools.insert(
            id,
            ShmPool { id, storage: ShmPoolStorage::FakeMemory(vec![0u8; size as usize]), size },
        );
        Ok(())
    }

    pub fn create_pool_from_fd(
        &mut self,
        id: WaylandObjectId,
        fd: crate::WireOwnedFd,
        size: u32,
    ) -> Result<()> {
        self.validate_new_pool(id, size)?;
        self.pools.insert(id, ShmPool { id, storage: ShmPoolStorage::ReceivedFd(fd), size });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
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

        if offset < 0 || stride <= 0 || height <= 0 || width <= 0 {
            return Err(WireError::InvalidSize(0));
        }
        if self.buffers.contains_key(&id) {
            return Err(WireError::ProtocolError("duplicate wl_shm buffer identity".into()));
        }
        if self.buffers.len() >= MAX_SHM_BUFFERS {
            return Err(WireError::ProtocolError("wl_shm buffer budget exhausted".into()));
        }

        if format == 0 || format == 1 {
            let minimum_stride =
                (width as u64).checked_mul(4).ok_or(WireError::InvalidSize(u32::MAX))?;
            if (stride as u64) < minimum_stride {
                return Err(WireError::ProtocolError("stride too small for format".into()));
            }
        }

        let byte_len =
            (stride as u64).checked_mul(height as u64).ok_or(WireError::InvalidSize(u32::MAX))?;
        if byte_len > MAX_SHM_BUFFER_BYTES as u64 {
            return Err(WireError::ProtocolError(format!(
                "wl_shm buffer exceeds {} bytes",
                MAX_SHM_BUFFER_BYTES
            )));
        }
        let total_size =
            (offset as u64).checked_add(byte_len).ok_or(WireError::InvalidSize(u32::MAX))?;
        if total_size > pool.size as u64 {
            return Err(WireError::InvalidSize(total_size.min(u64::from(u32::MAX)) as u32));
        }

        self.buffers.insert(id, ShmBuffer { id, pool_id, offset, width, height, stride, format });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_fake_pool_before_allocation() {
        let mut manager = ShmManager::default();
        let error = manager
            .create_pool_from_fake(
                WaylandObjectId(1),
                (MAX_FAKE_SHM_POOL_BYTES as u32).saturating_add(1),
            )
            .unwrap_err();
        assert!(matches!(error, WireError::ProtocolError(_)));
        assert!(manager.pools.is_empty());
    }

    #[test]
    fn rejects_buffer_size_and_arithmetic_overflow_before_registration() {
        let mut manager = ShmManager::default();
        manager.create_pool_from_fake(WaylandObjectId(1), 4096).unwrap();

        assert!(manager
            .create_buffer(WaylandObjectId(2), WaylandObjectId(1), 0, 1024, 1024, 4096, 1)
            .is_err());
        assert!(manager
            .create_buffer(
                WaylandObjectId(3),
                WaylandObjectId(1),
                i32::MAX,
                i32::MAX,
                i32::MAX,
                i32::MAX,
                1,
            )
            .is_err());
        assert!(manager.buffers.is_empty());
    }

    #[test]
    fn valid_small_buffer_has_exact_checked_byte_length() {
        let mut manager = ShmManager::default();
        manager.create_pool_from_fake(WaylandObjectId(1), 4096).unwrap();
        manager.create_buffer(WaylandObjectId(2), WaylandObjectId(1), 0, 16, 16, 64, 1).unwrap();
        assert_eq!(manager.buffers[&WaylandObjectId(2)].byte_len(), Some(1024));
    }
}
