use crate::{
    codec::{decode_message, encode_message},
    Result, WaylandMessage, WaylandObjectId, WaylandOpcode, WireError,
};
use byteorder::ByteOrder;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub struct WireFakeClient {
    stream: UnixStream,
}

impl WireFakeClient {
    pub fn connect<P: AsRef<Path>>(path: P) -> Result<Self> {
        let stream = UnixStream::connect(path)?;
        Ok(Self { stream })
    }

    pub fn send_message(&mut self, message: &WaylandMessage) -> Result<()> {
        let encoded = encode_message(message)?;
        self.stream.write_all(&encoded)?;
        Ok(())
    }

    pub fn receive_events(&mut self) -> Result<Vec<WaylandMessage>> {
        let mut buffer = vec![0u8; 4096];
        self.stream.set_read_timeout(Some(std::time::Duration::from_millis(100)))?;
        let n = match self.stream.read(&mut buffer) {
            Ok(n) => n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(Vec::new()),
            Err(e) => return Err(WireError::Io(e)),
        };

        if n == 0 {
            return Err(WireError::ConnectionClosed);
        }

        let mut events = Vec::new();
        let mut consumed = 0;
        while consumed < n {
            match decode_message(&buffer[consumed..n]) {
                Ok(msg) => {
                    consumed += msg.header.size as usize;
                    events.push(msg);
                }
                Err(WireError::Incomplete) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(events)
    }

    pub fn sync(&mut self, callback_id: u32) -> Result<()> {
        let mut payload = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut payload, callback_id);
        let msg = WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(0), payload);
        self.send_message(&msg)
    }

    pub fn get_registry(&mut self, registry_id: u32) -> Result<()> {
        let mut payload = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut payload, registry_id);
        let msg = WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(1), payload);
        self.send_message(&msg)
    }

    pub fn bind_wl_compositor(
        &mut self,
        registry_id: u32,
        name: u32,
        version: u32,
        new_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&name.to_le_bytes());
        crate::args::encode_string("wl_compositor", &mut p);
        p.extend_from_slice(&version.to_le_bytes());
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(registry_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn bind_wl_shm(
        &mut self,
        registry_id: u32,
        name: u32,
        version: u32,
        new_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&name.to_le_bytes());
        crate::args::encode_string("wl_shm", &mut p);
        p.extend_from_slice(&version.to_le_bytes());
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(registry_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn bind_xdg_wm_base(
        &mut self,
        registry_id: u32,
        name: u32,
        version: u32,
        new_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&name.to_le_bytes());
        crate::args::encode_string("xdg_wm_base", &mut p);
        p.extend_from_slice(&version.to_le_bytes());
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(registry_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn bind_wl_seat(
        &mut self,
        registry_id: u32,
        name: u32,
        version: u32,
        new_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&name.to_le_bytes());
        crate::args::encode_string("wl_seat", &mut p);
        p.extend_from_slice(&version.to_le_bytes());
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(registry_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn xdg_wm_base_get_xdg_surface(
        &mut self,
        wm_base_id: u32,
        new_id: u32,
        wl_surf_id: u32,
    ) -> Result<()> {
        let mut p = vec![0u8; 8];
        byteorder::LittleEndian::write_u32(&mut p[0..4], new_id);
        byteorder::LittleEndian::write_u32(&mut p[4..8], wl_surf_id);
        let msg = WaylandMessage::new(WaylandObjectId(wm_base_id), WaylandOpcode(3), p);
        self.send_message(&msg)
    }

    pub fn xdg_surface_get_toplevel(&mut self, xdg_surf_id: u32, new_id: u32) -> Result<()> {
        let mut p = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut p, new_id);
        let msg = WaylandMessage::new(WaylandObjectId(xdg_surf_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn xdg_surface_ack_configure(&mut self, xdg_surf_id: u32, serial: u32) -> Result<()> {
        let mut p = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut p, serial);
        let msg = WaylandMessage::new(WaylandObjectId(xdg_surf_id), WaylandOpcode(4), p);
        self.send_message(&msg)
    }

    pub fn xdg_toplevel_set_title(&mut self, xdg_top_id: u32, title: &str) -> Result<()> {
        let mut p = Vec::new();
        crate::args::encode_string(title, &mut p);
        let msg = WaylandMessage::new(WaylandObjectId(xdg_top_id), WaylandOpcode(2), p);
        self.send_message(&msg)
    }

    pub fn xdg_toplevel_set_app_id(&mut self, xdg_top_id: u32, app_id: &str) -> Result<()> {
        let mut p = Vec::new();
        crate::args::encode_string(app_id, &mut p);
        let msg = WaylandMessage::new(WaylandObjectId(xdg_top_id), WaylandOpcode(3), p);
        self.send_message(&msg)
    }

    pub fn wl_seat_get_pointer(&mut self, seat_id: u32, new_id: u32) -> Result<()> {
        let mut p = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut p, new_id);
        let msg = WaylandMessage::new(WaylandObjectId(seat_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn wl_seat_get_keyboard(&mut self, seat_id: u32, new_id: u32) -> Result<()> {
        let mut p = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut p, new_id);
        let msg = WaylandMessage::new(WaylandObjectId(seat_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn wl_compositor_create_surface(&mut self, compositor_id: u32, new_id: u32) -> Result<()> {
        let mut p = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut p, new_id);
        let msg = WaylandMessage::new(WaylandObjectId(compositor_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn wl_shm_create_pool(
        &mut self,
        shm_id: u32,
        new_id: u32,
        real_fd: std::os::unix::io::RawFd,
        size: i32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&new_id.to_le_bytes());
        p.extend_from_slice(&size.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(shm_id), WaylandOpcode(0), p);

        let encoded = encode_message(&msg)?;
        crate::fd::send_with_fds(&self.stream, &encoded, &[real_fd])?;
        Ok(())
    }

    pub fn wl_shm_pool_create_buffer(
        &mut self,
        pool_id: u32,
        new_id: u32,
        offset: i32,
        width: i32,
        height: i32,
        stride: i32,
        format: u32,
    ) -> Result<()> {
        let mut p = vec![0u8; 24];
        byteorder::LittleEndian::write_u32(&mut p[0..4], new_id);
        byteorder::LittleEndian::write_i32(&mut p[4..8], offset);
        byteorder::LittleEndian::write_i32(&mut p[8..12], width);
        byteorder::LittleEndian::write_i32(&mut p[12..16], height);
        byteorder::LittleEndian::write_i32(&mut p[16..20], stride);
        byteorder::LittleEndian::write_u32(&mut p[20..24], format);
        let msg = WaylandMessage::new(WaylandObjectId(pool_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn wl_surface_attach(
        &mut self,
        surface_id: u32,
        buffer_id: u32,
        x: i32,
        y: i32,
    ) -> Result<()> {
        let mut p = vec![0u8; 12];
        byteorder::LittleEndian::write_u32(&mut p[0..4], buffer_id);
        byteorder::LittleEndian::write_i32(&mut p[4..8], x);
        byteorder::LittleEndian::write_i32(&mut p[8..12], y);
        let msg = WaylandMessage::new(WaylandObjectId(surface_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn wl_surface_damage(
        &mut self,
        surface_id: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        let mut p = vec![0u8; 16];
        byteorder::LittleEndian::write_i32(&mut p[0..4], x);
        byteorder::LittleEndian::write_i32(&mut p[4..8], y);
        byteorder::LittleEndian::write_i32(&mut p[8..12], width);
        byteorder::LittleEndian::write_i32(&mut p[12..16], height);
        let msg = WaylandMessage::new(WaylandObjectId(surface_id), WaylandOpcode(2), p);
        self.send_message(&msg)
    }

    pub fn wl_surface_frame(&mut self, surface_id: u32, callback_id: u32) -> Result<()> {
        let mut p = vec![0u8; 4];
        byteorder::LittleEndian::write_u32(&mut p, callback_id);
        let msg = WaylandMessage::new(WaylandObjectId(surface_id), WaylandOpcode(3), p);
        self.send_message(&msg)
    }

    pub fn wl_surface_commit(&mut self, surface_id: u32) -> Result<()> {
        let msg = WaylandMessage::new(WaylandObjectId(surface_id), WaylandOpcode(6), vec![]);
        self.send_message(&msg)
    }
}

impl WireFakeClient {
    pub fn bind_wl_data_device_manager(
        &mut self,
        registry_id: u32,
        name: u32,
        version: u32,
        new_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&name.to_le_bytes());
        crate::args::encode_string("wl_data_device_manager", &mut p);
        p.extend_from_slice(&version.to_le_bytes());
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(registry_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn wl_data_device_manager_create_data_source(
        &mut self,
        manager_id: u32,
        new_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(manager_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn wl_data_device_manager_get_data_device(
        &mut self,
        manager_id: u32,
        new_id: u32,
        seat_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&new_id.to_le_bytes());
        p.extend_from_slice(&seat_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(manager_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn wl_data_source_offer(&mut self, source_id: u32, mime_type: &str) -> Result<()> {
        let mut p = Vec::new();
        crate::args::encode_string(mime_type, &mut p);
        let msg = WaylandMessage::new(WaylandObjectId(source_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn wl_data_source_destroy(&mut self, source_id: u32) -> Result<()> {
        let msg = WaylandMessage::new(WaylandObjectId(source_id), WaylandOpcode(1), Vec::new());
        self.send_message(&msg)
    }

    pub fn wl_data_device_set_selection(
        &mut self,
        device_id: u32,
        source_id: Option<u32>,
        serial: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&source_id.unwrap_or(0).to_le_bytes());
        p.extend_from_slice(&serial.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(device_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn wl_data_offer_receive(
        &mut self,
        offer_id: u32,
        mime_type: &str,
        fd: std::os::unix::io::RawFd,
    ) -> Result<()> {
        let mut p = Vec::new();
        crate::args::encode_string(mime_type, &mut p);
        // In this fake client, we assume FD is sent separately via ancillary data.
        // But our current send_message doesn't support FDs.
        // For testing purpose, we need a way to send FDs.
        let msg = WaylandMessage::new(WaylandObjectId(offer_id), WaylandOpcode(1), p);

        let encoded = crate::codec::encode_message(&msg)?;
        crate::fd::send_with_fds(&self.stream, &encoded, &[fd])?;
        Ok(())
    }
    pub fn wl_data_device_start_drag(
        &mut self,
        device_id: u32,
        source_id: Option<u32>,
        origin_id: u32,
        icon_id: Option<u32>,
        serial: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&source_id.unwrap_or(0).to_le_bytes());
        p.extend_from_slice(&origin_id.to_le_bytes());
        p.extend_from_slice(&icon_id.unwrap_or(0).to_le_bytes());
        p.extend_from_slice(&serial.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(device_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }
}

impl WireFakeClient {
    pub fn bind_wl_subcompositor(
        &mut self,
        registry_id: u32,
        name: u32,
        version: u32,
        new_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&name.to_le_bytes());
        crate::args::encode_string("wl_subcompositor", &mut p);
        p.extend_from_slice(&version.to_le_bytes());
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(registry_id), WaylandOpcode(0), p);
        self.send_message(&msg)
    }

    pub fn wl_subcompositor_get_subsurface(
        &mut self,
        subcomp_id: u32,
        new_id: u32,
        surface_id: u32,
        parent_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&new_id.to_le_bytes());
        p.extend_from_slice(&surface_id.to_le_bytes());
        p.extend_from_slice(&parent_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(subcomp_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn wl_subsurface_set_position(&mut self, subsurf_id: u32, x: i32, y: i32) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(subsurf_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn wl_subsurface_set_sync(&mut self, subsurf_id: u32) -> Result<()> {
        let msg = WaylandMessage::new(WaylandObjectId(subsurf_id), WaylandOpcode(4), Vec::new());
        self.send_message(&msg)
    }

    pub fn wl_subsurface_set_desync(&mut self, subsurf_id: u32) -> Result<()> {
        let msg = WaylandMessage::new(WaylandObjectId(subsurf_id), WaylandOpcode(5), Vec::new());
        self.send_message(&msg)
    }

    pub fn wl_subsurface_destroy(&mut self, subsurf_id: u32) -> Result<()> {
        let msg = WaylandMessage::new(WaylandObjectId(subsurf_id), WaylandOpcode(0), Vec::new());
        self.send_message(&msg)
    }

    pub fn xdg_wm_base_create_positioner(&mut self, manager_id: u32, new_id: u32) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&new_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(manager_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn xdg_positioner_set_size(&mut self, pos_id: u32, width: i32, height: i32) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&width.to_le_bytes());
        p.extend_from_slice(&height.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(pos_id), WaylandOpcode(1), p);
        self.send_message(&msg)
    }

    pub fn xdg_positioner_set_anchor_rect(
        &mut self,
        pos_id: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        p.extend_from_slice(&w.to_le_bytes());
        p.extend_from_slice(&h.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(pos_id), WaylandOpcode(2), p);
        self.send_message(&msg)
    }

    pub fn xdg_surface_get_popup(
        &mut self,
        xdg_surf_id: u32,
        new_id: u32,
        parent_id: Option<u32>,
        pos_id: u32,
    ) -> Result<()> {
        let mut p = Vec::new();
        p.extend_from_slice(&new_id.to_le_bytes());
        p.extend_from_slice(&parent_id.unwrap_or(0).to_le_bytes());
        p.extend_from_slice(&pos_id.to_le_bytes());
        let msg = WaylandMessage::new(WaylandObjectId(xdg_surf_id), WaylandOpcode(2), p);
        self.send_message(&msg)
    }

    pub fn xdg_popup_destroy(&mut self, popup_id: u32) -> Result<()> {
        let msg = WaylandMessage::new(WaylandObjectId(popup_id), WaylandOpcode(0), Vec::new());
        self.send_message(&msg)
    }
}
