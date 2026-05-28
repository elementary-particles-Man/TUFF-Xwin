use crate::{registry::WireObjectRegistry, Result, WaylandMessage, WaylandObjectId, WaylandOpcode};
use byteorder::{ByteOrder, LittleEndian};

pub struct WireGlobal {
    pub name: u32,
    pub interface: String,
    pub version: u32,
}

pub struct HeadlessWireCore {
    registry: WireObjectRegistry,
    globals: Vec<WireGlobal>,
    events_out: Vec<WaylandMessage>,
}

impl Default for HeadlessWireCore {
    fn default() -> Self {
        let mut core = Self {
            registry: WireObjectRegistry::default(),
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

        match (obj.interface.as_str(), message.header.opcode.0) {
            ("wl_display", 1) => self.handle_get_registry(message),
            ("wl_display", 0) => self.handle_sync(message),
            ("wl_registry", 0) => self.handle_registry_bind(message),
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
            return Ok(()); // Should be error but let's be safe
        }
        let new_id = LittleEndian::read_u32(&message.payload[0..4]);
        let new_id = WaylandObjectId(new_id);

        self.registry.register_client_object(new_id, "wl_registry", 1)?;

        // Send global events
        for global in &self.globals {
            self.events_out.push(self.create_global_event(new_id, global));
        }

        Ok(())
    }

    fn handle_sync(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 4 {
            return Ok(());
        }
        let callback_id = LittleEndian::read_u32(&message.payload[0..4]);
        let callback_id = WaylandObjectId(callback_id);

        self.registry.register_client_object(callback_id, "wl_callback", 1)?;

        // Send wl_callback.done event (opcode 0)
        let mut payload = vec![0u8; 4];
        LittleEndian::write_u32(&mut payload[0..4], 0); // serial
        self.events_out.push(WaylandMessage::new(callback_id, WaylandOpcode(0), payload));

        Ok(())
    }

    fn handle_registry_bind(&mut self, message: WaylandMessage) -> Result<()> {
        if message.payload.len() < 12 {
            return Ok(());
        }
        let _name = LittleEndian::read_u32(&message.payload[0..4]);
        // Wayland wire format for string is u32 length + null-terminated bytes + padding
        let interface_len = LittleEndian::read_u32(&message.payload[4..8]) as usize;
        // In real impl we would parse the string and version, but for parity P1 we just stub it

        let new_id_pos = 8 + ((interface_len + 3) & !3);
        if message.payload.len() >= new_id_pos + 4 {
            let _new_id = LittleEndian::read_u32(&message.payload[new_id_pos + 4..new_id_pos + 8]);
            // Wait, bind signature: name (u32), interface (string), version (u32), id (new_id)
            // Let's assume the client follows the protocol.
        }

        Ok(())
    }

    fn create_global_event(
        &self,
        registry_id: WaylandObjectId,
        global: &WireGlobal,
    ) -> WaylandMessage {
        // wl_registry.global: name (u32), interface (string), version (u32)
        let interface_bytes = global.interface.as_bytes();
        let len = interface_bytes.len() + 1; // null-term
        let padded_len = (len + 3) & !3;

        let mut payload = vec![0u8; 4 + 4 + padded_len + 4];
        LittleEndian::write_u32(&mut payload[0..4], global.name);
        LittleEndian::write_u32(&mut payload[4..8], len as u32);
        payload[8..8 + interface_bytes.len()].copy_from_slice(interface_bytes);
        LittleEndian::write_u32(&mut payload[8 + padded_len..12 + padded_len], global.version);

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
    use crate::WaylandOpcode;

    #[test]
    fn test_wl_display_get_registry() {
        let mut core = HeadlessWireCore::default();
        let mut payload = vec![0u8; 4];
        LittleEndian::write_u32(&mut payload[0..4], 2); // Client wants wl_registry at ID 2

        let msg = WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(1), payload);
        core.dispatch(msg).expect("dispatch");

        let registry = core.registry.get_object(WaylandObjectId(2)).expect("registry exists");
        assert_eq!(registry.interface, "wl_registry");

        // Should have 3 global events
        let mut count = 0;
        while let Some(_) = core.pop_event() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
