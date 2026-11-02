use crate::{Result, WaylandObjectId, WireError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireObject {
    pub id: WaylandObjectId,
    pub interface: String,
    pub version: u32,
}

pub struct WireObjectRegistry {
    objects: HashMap<WaylandObjectId, WireObject>,
    retired_pending_delete: HashSet<WaylandObjectId>,
    reusable_ids: HashSet<WaylandObjectId>,
    queued_delete_ids: BTreeSet<WaylandObjectId>,
    next_server_id: u32,
}

impl Default for WireObjectRegistry {
    fn default() -> Self {
        let mut registry = Self {
            objects: HashMap::new(),
            retired_pending_delete: HashSet::new(),
            reusable_ids: HashSet::new(),
            queued_delete_ids: BTreeSet::new(),
            next_server_id: 0xff000000,
        };

        // Pre-register wl_display (ID 1)
        registry.objects.insert(
            WaylandObjectId::DISPLAY,
            WireObject { id: WaylandObjectId::DISPLAY, interface: "wl_display".into(), version: 1 },
        );

        registry
    }
}

impl WireObjectRegistry {
    pub fn get_object(&self, id: WaylandObjectId) -> Result<&WireObject> {
        self.objects.get(&id).ok_or(WireError::InvalidObjectId(id.0))
    }

    pub fn register_client_object(
        &mut self,
        id: WaylandObjectId,
        interface: &str,
        version: u32,
    ) -> Result<()> {
        if id == WaylandObjectId::DISPLAY
            || self.objects.contains_key(&id)
            || self.retired_pending_delete.contains(&id)
        {
            return Err(WireError::InvalidObjectId(id.0));
        }

        self.objects.insert(id, WireObject { id, interface: interface.into(), version });
        self.reusable_ids.remove(&id);
        Ok(())
    }

    pub fn destroy_object(&mut self, id: WaylandObjectId) -> Result<()> {
        if self.objects.remove(&id).is_some() {
            if id.0 < 0xff000000 && id != WaylandObjectId::DISPLAY {
                self.retired_pending_delete.insert(id);
            }
            Ok(())
        } else {
            Err(WireError::InvalidObjectId(id.0))
        }
    }

    pub fn take_delete_id_events(&mut self) -> Vec<WaylandObjectId> {
        let ids: Vec<_> = self
            .retired_pending_delete
            .iter()
            .filter(|id| !self.queued_delete_ids.contains(id))
            .copied()
            .collect();
        let mut ids = ids;
        ids.sort_unstable();
        for id in &ids {
            self.queued_delete_ids.insert(*id);
        }
        ids
    }

    pub fn acknowledge_delete_id(&mut self, id: WaylandObjectId) -> Result<()> {
        if !self.retired_pending_delete.remove(&id) {
            return Err(WireError::InvalidObjectId(id.0));
        }
        self.queued_delete_ids.remove(&id);
        self.reusable_ids.insert(id);
        Ok(())
    }

    pub fn next_server_id(&mut self) -> WaylandObjectId {
        let id = WaylandObjectId(self.next_server_id);
        self.next_server_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_initial_state() {
        let registry = WireObjectRegistry::default();
        let display = registry.get_object(WaylandObjectId::DISPLAY).expect("wl_display exists");
        assert_eq!(display.interface, "wl_display");
    }

    #[test]
    fn test_register_and_destroy() {
        let mut registry = WireObjectRegistry::default();
        let id = WaylandObjectId(2);
        registry.register_client_object(id, "wl_registry", 1).expect("register");

        {
            let obj = registry.get_object(id).expect("exists");
            assert_eq!(obj.interface, "wl_registry");
        }

        registry.destroy_object(id).expect("destroy");
        assert!(registry.get_object(id).is_err());
    }

    #[test]
    fn test_forbid_reuse() {
        let mut registry = WireObjectRegistry::default();
        let id = WaylandObjectId(2);
        registry.register_client_object(id, "wl_registry", 1).expect("register");
        registry.destroy_object(id).expect("destroy");

        assert_eq!(registry.take_delete_id_events(), vec![id]);
        assert!(registry.register_client_object(id, "wl_registry", 1).is_err());
        registry.acknowledge_delete_id(id).expect("delete_id delivered");
        registry.register_client_object(id, "wl_registry", 1).expect("reuse after delete_id");
    }

    #[test]
    fn delete_id_is_terminal_and_duplicate_destroy_is_rejected() {
        let mut registry = WireObjectRegistry::default();
        let id = WaylandObjectId(7);
        registry.register_client_object(id, "wl_callback", 1).unwrap();
        registry.destroy_object(id).unwrap();
        assert!(registry.destroy_object(id).is_err());
        assert_eq!(registry.take_delete_id_events(), vec![id]);
        assert!(registry.take_delete_id_events().is_empty());
        assert!(registry.acknowledge_delete_id(WaylandObjectId(8)).is_err());
    }
}
