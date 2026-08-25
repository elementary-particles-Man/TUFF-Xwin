use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct PixelTransportHandle {
    pub client_id: u64,
    pub surface_id: String,
    pub buffer_generation: u64,
    pub scene_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PixelTransportPayload {
    pub handle: PixelTransportHandle,
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}

impl PixelTransportPayload {
    pub fn validate(&self) -> Result<(), PixelTransportError> {
        validate_payload(self)
    }

    pub fn expected_byte_len(&self) -> Result<usize, PixelTransportError> {
        expected_payload_len(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelTransportError {
    StaleGeneration { current: PixelTransportHandle, incoming: PixelTransportHandle },
    InvalidPayload { reason: String },
}

#[derive(Debug, Default)]
pub struct PixelTransportStore {
    payloads: BTreeMap<PixelTransportHandle, PixelTransportPayload>,
}

impl PixelTransportStore {
    pub fn submit(&mut self, payload: PixelTransportPayload) -> Result<(), PixelTransportError> {
        validate_payload(&payload)?;
        if let Some(current) = self
            .latest_handle_for_surface(payload.handle.client_id, payload.handle.surface_id.as_str())
        {
            if is_newer_handle(&current, &payload.handle) {
                return Err(PixelTransportError::StaleGeneration {
                    current,
                    incoming: payload.handle,
                });
            }
        }

        let handle = payload.handle.clone();
        self.payloads.retain(|existing, _| {
            !(existing.client_id == handle.client_id
                && existing.surface_id == handle.surface_id
                && !is_newer_handle(existing, &handle))
        });
        self.payloads.insert(handle, payload);
        Ok(())
    }

    pub fn lookup(&self, handle: &PixelTransportHandle) -> Option<&PixelTransportPayload> {
        self.payloads.get(handle)
    }

    pub fn replace_client(
        &mut self,
        client_id: u64,
        payloads: Vec<PixelTransportPayload>,
    ) -> Result<(), PixelTransportError> {
        let mut surface_ids = BTreeSet::new();
        for payload in &payloads {
            validate_payload(payload)?;
            if payload.handle.client_id != client_id {
                return Err(PixelTransportError::InvalidPayload {
                    reason: "replacement payload client identity mismatch".into(),
                });
            }
            if !surface_ids.insert(payload.handle.surface_id.as_str()) {
                return Err(PixelTransportError::InvalidPayload {
                    reason: format!(
                        "duplicate replacement payload for surface {}",
                        payload.handle.surface_id
                    ),
                });
            }
        }

        self.invalidate_client(client_id);
        for payload in payloads {
            self.payloads.insert(payload.handle.clone(), payload);
        }
        Ok(())
    }

    pub fn invalidate_client(&mut self, client_id: u64) {
        self.payloads.retain(|handle, _| handle.client_id != client_id);
    }

    pub fn len(&self) -> usize {
        self.payloads.len()
    }

    pub fn len_for_client(&self, client_id: u64) -> usize {
        self.payloads.keys().filter(|handle| handle.client_id == client_id).count()
    }

    pub fn total_bytes(&self) -> usize {
        self.payloads
            .values()
            .fold(0usize, |total, payload| total.saturating_add(payload.pixels.len()))
    }

    pub fn bytes_for_client(&self, client_id: u64) -> usize {
        self.payloads
            .iter()
            .filter(|(handle, _)| handle.client_id == client_id)
            .fold(0usize, |total, (_, payload)| total.saturating_add(payload.pixels.len()))
    }

    pub fn is_empty(&self) -> bool {
        self.payloads.is_empty()
    }

    fn latest_handle_for_surface(
        &self,
        client_id: u64,
        surface_id: &str,
    ) -> Option<PixelTransportHandle> {
        self.payloads
            .keys()
            .filter(|handle| handle.client_id == client_id && handle.surface_id == surface_id)
            .max_by(|left, right| compare_generation(left, right))
            .cloned()
    }
}

fn expected_payload_len(payload: &PixelTransportPayload) -> Result<usize, PixelTransportError> {
    if payload.width == 0 || payload.height == 0 {
        return Err(PixelTransportError::InvalidPayload {
            reason: "payload dimensions must be non-zero".into(),
        });
    }
    let minimum_stride = payload.width.checked_mul(4).ok_or_else(|| {
        PixelTransportError::InvalidPayload { reason: "payload stride calculation overflow".into() }
    })?;
    if payload.stride < minimum_stride {
        return Err(PixelTransportError::InvalidPayload {
            reason: "payload stride is smaller than 32-bit pixel width".into(),
        });
    }
    (payload.stride as usize).checked_mul(payload.height as usize).ok_or_else(|| {
        PixelTransportError::InvalidPayload { reason: "payload byte length overflow".into() }
    })
}

fn validate_payload(payload: &PixelTransportPayload) -> Result<(), PixelTransportError> {
    let required = expected_payload_len(payload)?;
    if payload.pixels.len() != required {
        return Err(PixelTransportError::InvalidPayload {
            reason: "payload byte length must equal stride * height".into(),
        });
    }
    Ok(())
}

fn is_newer_handle(left: &PixelTransportHandle, right: &PixelTransportHandle) -> bool {
    compare_generation(left, right).is_gt()
}

fn compare_generation(
    left: &PixelTransportHandle,
    right: &PixelTransportHandle,
) -> std::cmp::Ordering {
    (left.scene_generation, left.buffer_generation)
        .cmp(&(right.scene_generation, right.buffer_generation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(client_id: u64, surface_id: &str, buffer: u64, scene: u64) -> PixelTransportPayload {
        PixelTransportPayload {
            handle: PixelTransportHandle {
                client_id,
                surface_id: surface_id.into(),
                buffer_generation: buffer,
                scene_generation: scene,
            },
            pixels: vec![0x11; 16],
            width: 2,
            height: 2,
            stride: 8,
            format: 1,
        }
    }

    #[test]
    fn submits_and_resolves_exact_generation_bound_payload() {
        let mut store = PixelTransportStore::default();
        let submitted = payload(1, "surface-7", 3, 9);
        let handle = submitted.handle.clone();

        store.submit(submitted).unwrap();

        assert_eq!(store.lookup(&handle).unwrap().width, 2);
        assert!(store.lookup(&PixelTransportHandle { buffer_generation: 4, ..handle }).is_none());
    }

    #[test]
    fn supersedes_older_payload_for_same_client_surface() {
        let mut store = PixelTransportStore::default();
        let old = payload(1, "surface-7", 3, 9);
        let old_handle = old.handle.clone();
        let new = payload(1, "surface-7", 4, 9);
        let new_handle = new.handle.clone();

        store.submit(old).unwrap();
        store.submit(new).unwrap();

        assert!(store.lookup(&old_handle).is_none());
        assert!(store.lookup(&new_handle).is_some());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn rejects_stale_generation_for_same_client_surface() {
        let mut store = PixelTransportStore::default();
        store.submit(payload(1, "surface-7", 4, 10)).unwrap();

        let err = store.submit(payload(1, "surface-7", 3, 10)).unwrap_err();

        assert!(matches!(err, PixelTransportError::StaleGeneration { .. }));
    }

    #[test]
    fn invalidates_only_the_matching_client() {
        let mut store = PixelTransportStore::default();
        let keep = payload(2, "surface-7", 1, 1);
        let keep_handle = keep.handle.clone();
        store.submit(payload(1, "surface-7", 1, 1)).unwrap();
        store.submit(keep).unwrap();

        store.invalidate_client(1);

        assert_eq!(store.len(), 1);
        assert!(store.lookup(&keep_handle).is_some());
    }

    #[test]
    fn replacement_is_transactional_for_invalid_payloads() {
        let mut store = PixelTransportStore::default();
        let original = payload(1, "surface-7", 3, 9);
        let original_handle = original.handle.clone();
        store.submit(original).unwrap();

        let mut invalid = payload(1, "surface-8", 4, 10);
        invalid.pixels.pop();
        assert!(store.replace_client(1, vec![invalid]).is_err());

        assert!(store.lookup(&original_handle).is_some());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn replacement_does_not_duplicate_client_payload_bytes() {
        let mut store = PixelTransportStore::default();
        store.submit(payload(1, "surface-7", 1, 1)).unwrap();
        store.submit(payload(2, "surface-8", 1, 1)).unwrap();

        let replacement = payload(1, "surface-9", 2, 2);
        let replacement_handle = replacement.handle.clone();
        store.replace_client(1, vec![replacement]).unwrap();

        assert_eq!(store.len(), 2);
        assert_eq!(store.len_for_client(1), 1);
        assert_eq!(store.bytes_for_client(1), 16);
        assert_eq!(store.total_bytes(), 32);
        assert!(store.lookup(&replacement_handle).is_some());
    }

    #[test]
    fn exact_payload_length_is_required() {
        let mut too_large = payload(1, "surface-7", 1, 1);
        too_large.pixels.push(0);
        assert!(too_large.validate().is_err());

        let mut too_small = payload(1, "surface-7", 1, 1);
        too_small.pixels.pop();
        assert!(too_small.validate().is_err());
    }
}
