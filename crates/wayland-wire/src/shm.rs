use crate::{Result, WaylandObjectId, WireError};
use std::collections::{HashMap, HashSet};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

pub const MAX_SHM_BUFFER_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_FAKE_SHM_POOL_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_FAKE_SHM_TOTAL_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_SHM_POOLS: usize = 256;
pub const MAX_SHM_BUFFERS: usize = 4096;

pub enum ShmPoolStorage {
    FakeMemory(Vec<u8>),
    ReceivedFd(crate::WireOwnedFd),
}
pub type ShmBacking = Arc<Mutex<ShmPoolStorage>>;

pub struct ShmPool {
    pub id: WaylandObjectId,
    pub backing: ShmBacking,
    pub size: u32,
}

pub struct ShmBuffer {
    pub id: WaylandObjectId,
    pub pool_id: WaylandObjectId,
    pub backing: ShmBacking,
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
    fn fake_backing_bytes(&self) -> Result<usize> {
        let mut seen = HashSet::new();
        let mut total = 0usize;
        let mut refs = self.pools.values().map(|p| &p.backing).collect::<Vec<_>>();
        refs.extend(self.buffers.values().map(|b| &b.backing));
        for backing in refs {
            if !seen.insert(Arc::as_ptr(backing) as usize) {
                continue;
            }
            let storage = backing
                .lock()
                .map_err(|_| WireError::ProtocolError("SHM backing mutex poisoned".into()))?;
            if let ShmPoolStorage::FakeMemory(bytes) = &*storage {
                total = total.checked_add(bytes.len()).ok_or_else(|| {
                    WireError::ProtocolError("fake wl_shm byte accounting overflow".into())
                })?;
            }
        }
        Ok(total)
    }
    pub fn create_pool_from_fake(&mut self, id: WaylandObjectId, size: u32) -> Result<()> {
        self.validate_new_pool(id, size)?;
        if size as usize > MAX_FAKE_SHM_POOL_BYTES {
            return Err(WireError::ProtocolError(format!(
                "fake wl_shm pool exceeds {} bytes",
                MAX_FAKE_SHM_POOL_BYTES
            )));
        }
        let total = self.fake_backing_bytes()?.checked_add(size as usize).ok_or_else(|| {
            WireError::ProtocolError("fake wl_shm byte accounting overflow".into())
        })?;
        if total > MAX_FAKE_SHM_TOTAL_BYTES {
            return Err(WireError::ProtocolError(format!(
                "aggregate fake wl_shm pools exceed {} bytes",
                MAX_FAKE_SHM_TOTAL_BYTES
            )));
        }
        self.pools.insert(
            id,
            ShmPool {
                id,
                backing: Arc::new(Mutex::new(ShmPoolStorage::FakeMemory(vec![0; size as usize]))),
                size,
            },
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
        self.pools.insert(
            id,
            ShmPool { id, backing: Arc::new(Mutex::new(ShmPoolStorage::ReceivedFd(fd))), size },
        );
        Ok(())
    }
    pub fn destroy_pool(&mut self, id: WaylandObjectId) -> Result<()> {
        self.pools.remove(&id).ok_or(WireError::InvalidObjectId(id.0)).map(|_| ())
    }
    pub fn resize_pool(&mut self, id: WaylandObjectId, requested_size: i32) -> Result<()> {
        let pool = self.pools.get(&id).ok_or(WireError::InvalidObjectId(id.0))?;
        if requested_size <= 0 || requested_size as u32 <= pool.size {
            return Err(WireError::InvalidSize(requested_size.max(0) as u32));
        }
        let new_size = requested_size as u32;
        if new_size as usize > MAX_FAKE_SHM_POOL_BYTES {
            return Err(WireError::ProtocolError("fake wl_shm pool resize exceeds limit".into()));
        }
        let current_fake_total = self.fake_backing_bytes()?;
        let current_pool_size = pool.size as usize;
        let backing = Arc::clone(&pool.backing);
        let mut storage = backing
            .lock()
            .map_err(|_| WireError::ProtocolError("SHM backing mutex poisoned".into()))?;
        match &mut *storage {
            ShmPoolStorage::FakeMemory(bytes) => {
                let total = current_fake_total
                    .checked_sub(current_pool_size)
                    .and_then(|v| v.checked_add(new_size as usize))
                    .ok_or_else(|| {
                        WireError::ProtocolError("fake wl_shm byte accounting overflow".into())
                    })?;
                if total > MAX_FAKE_SHM_TOTAL_BYTES {
                    return Err(WireError::ProtocolError(
                        "aggregate fake wl_shm pools exceed limit".into(),
                    ));
                }
                bytes.resize(new_size as usize, 0);
            }
            ShmPoolStorage::ReceivedFd(fd) => {
                let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
                if unsafe { libc::fstat(fd.0.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
                    return Err(WireError::Io(std::io::Error::last_os_error()));
                }
                if new_size as u64 > unsafe { stat.assume_init().st_size as u64 } {
                    return Err(WireError::InvalidSize(new_size));
                }
            }
        }
        drop(storage);
        self.pools.get_mut(&id).expect("pool remains registered").size = new_size;
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
        let backing = Arc::clone(&pool.backing);
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
            let min = (width as u64).checked_mul(4).ok_or(WireError::InvalidSize(u32::MAX))?;
            if (stride as u64) < min {
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
        let total =
            (offset as u64).checked_add(byte_len).ok_or(WireError::InvalidSize(u32::MAX))?;
        if total > pool.size as u64 {
            return Err(WireError::InvalidSize(total.min(u64::from(u32::MAX)) as u32));
        }
        self.buffers
            .insert(id, ShmBuffer { id, pool_id, backing, offset, width, height, stride, format });
        Ok(())
    }
    pub fn destroy_buffer(&mut self, id: WaylandObjectId) -> Result<()> {
        self.buffers.remove(&id).ok_or(WireError::InvalidObjectId(id.0)).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversized_fake_pool_before_allocation() {
        let mut m = ShmManager::default();
        assert!(m
            .create_pool_from_fake(WaylandObjectId(1), (MAX_FAKE_SHM_POOL_BYTES as u32) + 1)
            .is_err());
        assert!(m.pools.is_empty());
    }
    #[test]
    fn rejects_buffer_size_and_arithmetic_overflow_before_registration() {
        let mut m = ShmManager::default();
        m.create_pool_from_fake(WaylandObjectId(1), 4096).unwrap();
        assert!(m
            .create_buffer(WaylandObjectId(2), WaylandObjectId(1), 0, 1024, 1024, 4096, 1)
            .is_err());
        assert!(m.buffers.is_empty());
    }
    #[test]
    fn valid_small_buffer_has_exact_checked_byte_length() {
        let mut m = ShmManager::default();
        m.create_pool_from_fake(WaylandObjectId(1), 4096).unwrap();
        m.create_buffer(WaylandObjectId(2), WaylandObjectId(1), 0, 16, 16, 64, 1).unwrap();
        assert_eq!(m.buffers[&WaylandObjectId(2)].byte_len(), Some(1024));
    }
    #[test]
    fn destroy_pool_preserves_backing_and_resize_zero_fills() {
        let mut m = ShmManager::default();
        m.create_pool_from_fake(WaylandObjectId(1), 16).unwrap();
        m.create_buffer(WaylandObjectId(2), WaylandObjectId(1), 0, 2, 2, 8, 1).unwrap();
        {
            let p = m.pools[&WaylandObjectId(1)].backing.lock().unwrap();
            drop(p);
        }
        m.resize_pool(WaylandObjectId(1), 32).unwrap();
        m.destroy_pool(WaylandObjectId(1)).unwrap();
        assert!(m.buffers.contains_key(&WaylandObjectId(2)));
        let b = m.buffers[&WaylandObjectId(2)].backing.lock().unwrap();
        assert!(matches!(&*b,ShmPoolStorage::FakeMemory(v) if v.len()==32));
    }
    #[test]
    fn resize_rejects_non_growth() {
        let mut m = ShmManager::default();
        m.create_pool_from_fake(WaylandObjectId(1), 16).unwrap();
        for s in [16, 0, -1] {
            assert!(m.resize_pool(WaylandObjectId(1), s).is_err());
        }
    }
}
