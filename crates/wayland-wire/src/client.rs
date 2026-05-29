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
            return Ok(Vec::new());
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
        _fake_fd: u32,
        size: i32,
    ) -> Result<()> {
        let mut p = vec![0u8; 8];
        byteorder::LittleEndian::write_u32(&mut p[0..4], new_id);
        byteorder::LittleEndian::write_i32(&mut p[4..8], size);
        let msg = WaylandMessage::new(WaylandObjectId(shm_id), WaylandOpcode(0), p);
        self.send_message(&msg)
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
