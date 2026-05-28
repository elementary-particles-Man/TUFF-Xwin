use crate::{
    registry::WireObjectRegistry,
    shm::ShmManager,
    surface::{Rect, SurfaceManager},
    FakeFd, Result, WaylandMessage, WaylandObjectId, WaylandOpcode, WireError,
};
use byteorder::{ByteOrder, LittleEndian};

pub struct WireGlobal {
    pub name: u32,
    pub interface: String,
    pub version: u32,
}

pub struct HeadlessWireCore {
    pub registry: WireObjectRegistry,
    pub surfaces: SurfaceManager,
    pub shm: ShmManager,
    globals: Vec<WireGlobal>,
    events_out: Vec<WaylandMessage>,
}

impl Default for HeadlessWireCore {
    fn default() -> Self {
        let mut core = Self {
            registry: WireObjectRegistry::default(),
            surfaces: SurfaceManager::new(),
            shm: ShmManager::new(),
            globals: Vec::new(),
            events_out: Vec::new(),
        };

        // Standard globals
        core.globals.push(WireGlobal { name: 1, interface: "wl_compositor".into(), version: 4 });
        core.globals.push(WireGlobal { name: 2, interface: "wl_shm".into(), version: 1 });
        core.globals.push(WireGlobal { name: 3, interface: "wl_seat".into(), version: 7 });

        core
    }
}

impl HeadlessWireCore {
    pub fn dispatch(&mut self, message: WaylandMessage) -> Result<()> {
        let obj = self.registry.get_object(message.header.object_id)?;
        let spec = crate::generated::core_protocol_spec();
        let iface_spec = spec.interfaces.get(&obj.interface).ok_or_else(|| {
            WireError::ProtocolError(format!("unknown interface: {}", obj.interface))
        })?;

        let msg_spec =
            iface_spec.requests.get(message.header.opcode.0 as usize).ok_or_else(|| {
                WireError::ProtocolError(format!(
                    "unknown opcode {} for {}",
                    message.header.opcode.0, obj.interface
                ))
            })?;

        // Validate arguments
        let args = crate::codec::decode_arguments(&message.payload, msg_spec)?;
        if !crate::signature::validate_args(msg_spec, &args) {
            return Err(WireError::ProtocolError(format!(
                "argument mismatch for {} opcode {}",
                obj.interface, message.header.opcode.0
            )));
        }

        match (obj.interface.as_str(), message.header.opcode.0) {
            ("wl_display", 1) => self.handle_get_registry(message),
            ("wl_display", 0) => self.handle_sync(message),
            ("wl_registry", 0) => self.handle_registry_bind(message),
            ("wl_compositor", 0) => self.handle_create_surface(message),
            ("wl_compositor", 1) => self.handle_create_region(message),
            ("wl_surface", 0) => self.handle_surface_destroy(message),
            ("wl_surface", 1) => self.handle_surface_attach(message),
            ("wl_surface", 2) => self.handle_surface_damage(message),
            ("wl_surface", 3) => self.handle_surface_frame(message),
            ("wl_surface", 6) => self.handle_surface_commit(message),
            ("wl_shm", 0) => self.handle_shm_create_pool(message),
            ("wl_shm_pool", 0) => self.handle_shm_pool_create_buffer(message),
            _ => {
                println!(
                    "warning: unhandled dispatch for {} (id={}) opcode={}",
                    obj.interface, message.header.object_id.0, message.header.opcode.0
                );
                Ok(())
            }
        }
    }

    fn handle_get_registry(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 4 {
            return Err(WireError::Incomplete);
        }
        let new_id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        self.registry.register_client_object(new_id, "wl_registry", 1)?;
        for global in &self.globals {
            self.events_out.push(self.create_global_event(new_id, global));
        }
        Ok(())
    }

    fn handle_sync(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 4 {
            return Err(WireError::Incomplete);
        }
        let callback_id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        self.registry.register_client_object(callback_id, "wl_callback", 1)?;
        let mut payload = vec![0u8; 4];
        LittleEndian::write_u32(&mut payload[0..4], 0); // serial
        self.events_out.push(WaylandMessage::new(callback_id, WaylandOpcode(0), payload));
        Ok(())
    }

    fn handle_registry_bind(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 12 {
            return Err(WireError::Incomplete);
        }
        let name = LittleEndian::read_u32(&message.payload[0..4]);

        // Simplified bind for P2: assume id is at offset 8 if payload is short,
        // or try to find it after the interface string.
        let new_id = if message.payload.len() == 12 {
            WaylandObjectId(LittleEndian::read_u32(&message.payload[8..12]))
        } else {
            let interface_len = LittleEndian::read_u32(&message.payload[4..8]) as usize;
            let padded_interface_len = (interface_len + 3) & !3;
            let pos_new_id = 8 + padded_interface_len + 4;
            if message.payload.len() < pos_new_id + 4 {
                return Err(WireError::Incomplete);
            }
            WaylandObjectId(LittleEndian::read_u32(&message.payload[pos_new_id..pos_new_id + 4]))
        };

        let global =
            self.globals.iter().find(|g| g.name == name).ok_or(WireError::InvalidObjectId(name))?;
        self.registry.register_client_object(new_id, &global.interface, global.version)?;

        if global.interface == "wl_shm" {
            self.send_shm_formats(new_id);
        }
        Ok(())
    }

    fn handle_create_surface(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 4 {
            return Err(WireError::Incomplete);
        }
        let id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        self.registry.register_client_object(id, "wl_surface", 4)?;
        self.surfaces.create_surface(id);
        Ok(())
    }

    fn handle_create_region(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 4 {
            return Err(WireError::Incomplete);
        }
        let id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        self.registry.register_client_object(id, "wl_region", 1)?;
        self.surfaces.create_region(id);
        Ok(())
    }

    fn handle_surface_destroy(&mut self, message: WaylandMessage) -> Result<()> {
        self.registry.destroy_object(message.header.object_id)
    }

    fn handle_surface_attach(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 12 {
            return Err(WireError::Incomplete);
        }
        let buffer_id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        let x = LittleEndian::read_i32(&message.payload[4..8]);
        let y = LittleEndian::read_i32(&message.payload[8..12]);
        if let Some(surface) = self.surfaces.surfaces.get_mut(&message.header.object_id) {
            surface.pending.buffer_id = if buffer_id.0 == 0 { None } else { Some(buffer_id) };
            surface.pending.offset_x = x;
            surface.pending.offset_y = y;
        }
        Ok(())
    }

    fn handle_surface_damage(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 16 {
            return Err(WireError::Incomplete);
        }
        let x = LittleEndian::read_i32(&message.payload[0..4]);
        let y = LittleEndian::read_i32(&message.payload[4..8]);
        let width = LittleEndian::read_u32(&message.payload[8..12]);
        let height = LittleEndian::read_u32(&message.payload[12..16]);
        if let Some(surface) = self.surfaces.surfaces.get_mut(&message.header.object_id) {
            surface.pending.damage.push(Rect { x, y, width, height });
        }
        Ok(())
    }

    fn handle_surface_frame(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 4 {
            return Err(WireError::Incomplete);
        }
        let callback_id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        self.registry.register_client_object(callback_id, "wl_callback", 1)?;
        if let Some(surface) = self.surfaces.surfaces.get_mut(&message.header.object_id) {
            surface.callbacks.push(callback_id);
        }
        Ok(())
    }

    fn handle_surface_commit(&mut self, message: WaylandMessage) -> Result<()> {
        self.surfaces.commit(message.header.object_id);
        Ok(())
    }

    fn handle_shm_create_pool(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 8 {
            return Err(WireError::Incomplete);
        }
        let id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        // Note: FD is not in payload, but passed via ancillary data.
        // For P2 headless core, we simulate by assuming it was passed.
        let size = LittleEndian::read_u32(&message.payload[4..8]);
        self.registry.register_client_object(id, "wl_shm_pool", 1)?;
        self.shm.create_pool(id, FakeFd(0), size);
        Ok(())
    }

    fn handle_shm_pool_create_buffer(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 24 {
            return Err(WireError::Incomplete);
        }
        let id = WaylandObjectId(LittleEndian::read_u32(&message.payload[0..4]));
        let offset = LittleEndian::read_i32(&message.payload[4..8]);
        let width = LittleEndian::read_i32(&message.payload[8..12]);
        let height = LittleEndian::read_i32(&message.payload[12..16]);
        let stride = LittleEndian::read_i32(&message.payload[16..20]);
        let format = LittleEndian::read_u32(&message.payload[20..24]);

        self.registry.register_client_object(id, "wl_buffer", 1)?;
        self.shm.create_buffer(id, message.header.object_id, offset, width, height, stride, format)
    }

    fn send_shm_formats(&mut self, shm_id: WaylandObjectId) {
        // wl_shm.format: Argb8888 (0), Xrgb8888 (1)
        for f in [0u32, 1u32] {
            let mut payload = vec![0u8; 4];
            LittleEndian::write_u32(&mut payload[0..4], f);
            self.events_out.push(WaylandMessage::new(shm_id, WaylandOpcode(0), payload));
        }
    }

    fn create_global_event(
        &self,
        registry_id: WaylandObjectId,
        global: &WireGlobal,
    ) -> WaylandMessage {
        let interface_bytes = global.interface.as_bytes();
        let len = (interface_bytes.len() + 1) as u32;
        let padded_len = (len + 3) & !3;
        let mut payload = vec![0u8; 4 + 4 + padded_len as usize + 4];
        LittleEndian::write_u32(&mut payload[0..4], global.name);
        LittleEndian::write_u32(&mut payload[4..8], len);
        payload[8..8 + interface_bytes.len()].copy_from_slice(interface_bytes);
        LittleEndian::write_u32(
            &mut payload[8 + padded_len as usize..12 + padded_len as usize],
            global.version,
        );
        WaylandMessage::new(registry_id, WaylandOpcode(0), payload)
    }

    pub fn pop_event(&mut self) -> Option<WaylandMessage> {
        if self.events_out.is_empty() {
            None
        } else {
            Some(self.events_out.remove(0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_surface_and_region() {
        let mut core = HeadlessWireCore::default();

        let mut p1 = vec![0u8; 4];
        LittleEndian::write_u32(&mut p1, 10);
        core.dispatch(WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(1), p1))
            .unwrap();

        // Bind wl_compositor (assume name 1)
        // Signature: name (u32), interface (string), version (u32), id (new_id)
        let mut p2 = Vec::new();
        p2.extend_from_slice(&1u32.to_le_bytes()); // name
        crate::args::encode_string("wl_compositor", &mut p2); // interface
        p2.extend_from_slice(&4u32.to_le_bytes()); // version
        p2.extend_from_slice(&11u32.to_le_bytes()); // new_id

        core.dispatch(WaylandMessage::new(WaylandObjectId(10), WaylandOpcode(0), p2))
            .unwrap();

        assert!(core.registry.get_object(WaylandObjectId(11)).is_ok());
        assert_eq!(
            core.registry.get_object(WaylandObjectId(11)).unwrap().interface,
            "wl_compositor"
        );
    }

}
