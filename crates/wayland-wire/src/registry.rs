use crate::{Result, WaylandObjectId, WireError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireObject {
    pub id: WaylandObjectId,
    pub interface: String,
    pub version: u32,
}

pub struct WireObjectRegistry {
    objects: HashMap<WaylandObjectId, WireObject>,
    used_ids: HashSet<WaylandObjectId>,
    next_server_id: u32,
}

impl Default for WireObjectRegistry {
    fn default() -> Self {
        let mut registry =
            Self { objects: HashMap::new(), used_ids: HashSet::new(), next_server_id: 0xff000000 };

        // Pre-register wl_display (ID 1)
        registry.objects.insert(
            WaylandObjectId::DISPLAY,
            WireObject { id: WaylandObjectId::DISPLAY, interface: "wl_display".into(), version: 1 },
        );
        registry.used_ids.insert(WaylandObjectId::DISPLAY);

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
        if self.used_ids.contains(&id) {
            // Technically Wayland allows reuse after a period, but task says reuse is forbidden.
            return Err(WireError::InvalidObjectId(id.0));
        }

        self.objects.insert(id, WireObject { id, interface: interface.into(), version });
        self.used_ids.insert(id);
        Ok(())
    }

    pub fn destroy_object(&mut self, id: WaylandObjectId) -> Result<()> {
        if self.objects.remove(&id).is_some() {
            Ok(())
        } else {
            Err(WireError::InvalidObjectId(id.0))
        }
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

        // Re-registering the same ID should fail based on task requirements
        assert!(registry.register_client_object(id, "wl_registry", 1).is_err());
    }
}
