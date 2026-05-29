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
}
