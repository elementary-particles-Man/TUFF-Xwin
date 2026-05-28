use crate::{Result, WaylandHeader, WaylandMessage, WaylandObjectId, WaylandOpcode, WireError};
use byteorder::{ByteOrder, LittleEndian};

pub fn decode_header(bytes: &[u8]) -> Result<WaylandHeader> {
    if bytes.len() < 8 {
        return Err(WireError::Incomplete);
    }

    let object_id = LittleEndian::read_u32(&bytes[0..4]);
    let second_word = LittleEndian::read_u32(&bytes[4..8]);
    let opcode = (second_word & 0xFFFF) as u16;
    let size = (second_word >> 16) as u16;

    if size < 8 {
        return Err(WireError::InvalidSize(size as u32));
    }

    Ok(WaylandHeader { object_id: WaylandObjectId(object_id), size, opcode: WaylandOpcode(opcode) })
}

pub fn decode_message(bytes: &[u8]) -> Result<WaylandMessage> {
    let header = decode_header(bytes)?;
    let size = header.size as usize;

    if bytes.len() < size {
        return Err(WireError::Incomplete);
    }

    let payload = bytes[8..size].to_vec();
    Ok(WaylandMessage { header, payload })
}

pub fn encode_message(message: &WaylandMessage) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; message.header.size as usize];

    LittleEndian::write_u32(&mut bytes[0..4], message.header.object_id.0);

    let second_word = (message.header.opcode.0 as u32) | ((message.header.size as u32) << 16);
    LittleEndian::write_u32(&mut bytes[4..8], second_word);

    bytes[8..].copy_from_slice(&message.payload);

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_roundtrip() {
        let msg = WaylandMessage::new(WaylandObjectId(1), WaylandOpcode(0), vec![1, 2, 3, 4]);
        let encoded = encode_message(&msg).expect("encode");
        assert_eq!(encoded.len(), 12);

        let decoded = decode_message(&encoded).expect("decode");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn test_reject_short_input() {
        let bytes = vec![0u8; 7];
        match decode_header(&bytes) {
            Err(WireError::Incomplete) => (),
            other => panic!("Expected Incomplete, got {:?}", other),
        }
    }

    #[test]
    fn test_reject_invalid_size() {
        let mut bytes = vec![0u8; 8];
        // Size 4 (too small)
        LittleEndian::write_u32(&mut bytes[4..8], 0x0004_0000);
        match decode_header(&bytes) {
            Err(WireError::InvalidSize(4)) => (),
            other => panic!("Expected InvalidSize(4), got {:?}", other),
        }
    }
}
