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

use crate::protocol::MessageSpec;
use crate::WireArg;

pub fn decode_arguments(
    bytes: &[u8],
    spec: &MessageSpec,
    total_fds: Option<usize>,
) -> Result<Vec<WireArg>> {
    let mut offset = 0;
    let mut args = Vec::with_capacity(spec.args.len());
    let mut fds_needed = 0;

    for arg_spec in &spec.args {
        match arg_spec.arg_type.as_str() {
            "int" => {
                if bytes.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                args.push(WireArg::Int(LittleEndian::read_i32(&bytes[offset..offset + 4])));
                offset += 4;
            }
            "uint" => {
                if bytes.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                args.push(WireArg::Uint(LittleEndian::read_u32(&bytes[offset..offset + 4])));
                offset += 4;
            }
            "fixed" => {
                if bytes.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                args.push(WireArg::Fixed(LittleEndian::read_i32(&bytes[offset..offset + 4])));
                offset += 4;
            }
            "string" => {
                let s = crate::args::decode_string(bytes, &mut offset)?;
                args.push(WireArg::String(s));
            }
            "object" => {
                if bytes.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                args.push(WireArg::Object(LittleEndian::read_u32(&bytes[offset..offset + 4])));
                offset += 4;
            }
            "new_id" => {
                if bytes.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                args.push(WireArg::NewId(LittleEndian::read_u32(&bytes[offset..offset + 4])));
                offset += 4;
            }
            "array" => {
                let a = crate::args::decode_array(bytes, &mut offset)?;
                args.push(WireArg::Array(a));
            }
            "fd" => {
                fds_needed += 1;
                match total_fds {
                    Some(_) => args.push(WireArg::AncillaryFd),
                    None => args.push(WireArg::Fd(crate::FakeFd(0))),
                }
            }
            _ => {
                return Err(WireError::ProtocolError(format!(
                    "unknown arg type: {}",
                    arg_spec.arg_type
                )))
            }
        }
    }

    if let Some(total) = total_fds {
        if fds_needed > total {
            return Err(WireError::ProtocolError("missing ancillary FD".into()));
        }
    }

    Ok(args)
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
