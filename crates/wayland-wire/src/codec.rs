use crate::{
    args::{encode_array, encode_string, WireArg},
    protocol::MessageSpec,
    registry::WireObjectRegistry,
    Result, WaylandHeader, WaylandMessage, WaylandObjectId, WaylandOpcode, WireError,
};
use byteorder::{ByteOrder, LittleEndian};

pub const MAX_WIRE_MESSAGE_BYTES: usize = 64 * 1024;

pub fn decode_header(bytes: &[u8]) -> Result<WaylandHeader> {
    if bytes.len() < 8 {
        return Err(WireError::Incomplete);
    }
    let object_id = WaylandObjectId(LittleEndian::read_u32(&bytes[0..4]));
    let word1 = LittleEndian::read_u32(&bytes[4..8]);
    let size = (word1 >> 16) as u16;
    let opcode = WaylandOpcode((word1 & 0xffff) as u16);

    Ok(WaylandHeader { object_id, opcode, size })
}

pub fn decode_message(bytes: &[u8]) -> Result<WaylandMessage> {
    let header = decode_header(bytes)?;
    if header.size < 8 || header.size as usize > MAX_WIRE_MESSAGE_BYTES {
        return Err(WireError::InvalidSize(header.size as u32));
    }

    if bytes.len() < header.size as usize {
        return Err(WireError::Incomplete);
    }
    let payload = bytes[8..header.size as usize].to_vec();
    Ok(WaylandMessage { header, payload })
}

pub fn decode_arguments(
    payload: &[u8],
    spec: &MessageSpec,
    total_fds: Option<usize>,
) -> Result<Vec<WireArg>> {
    let mut args = Vec::with_capacity(spec.args.len());
    let mut offset = 0;
    let mut fd_count = 0;

    for arg_spec in &spec.args {
        match arg_spec.arg_type.as_str() {
            "int" => {
                if payload.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                args.push(WireArg::Int(LittleEndian::read_i32(&payload[offset..offset + 4])));
                offset += 4;
            }
            "uint" | "object" | "new_id" => {
                if payload.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                let val = LittleEndian::read_u32(&payload[offset..offset + 4]);
                match arg_spec.arg_type.as_str() {
                    "uint" => args.push(WireArg::Uint(val)),
                    "object" => args.push(WireArg::Object(val)),
                    "new_id" => args.push(WireArg::NewId(val)),
                    _ => unreachable!(),
                }
                offset += 4;
            }
            "fixed" => {
                if payload.len() < offset + 4 {
                    return Err(WireError::Incomplete);
                }
                args.push(WireArg::Fixed(LittleEndian::read_i32(&payload[offset..offset + 4])));
                offset += 4;
            }
            "string" => {
                args.push(WireArg::String(crate::args::decode_string(payload, &mut offset)?));
            }
            "array" => {
                args.push(WireArg::Array(crate::args::decode_array(payload, &mut offset)?));
            }
            "fd" => {
                if let Some(total) = total_fds {
                    if fd_count >= total {
                        return Err(WireError::ProtocolError(
                            "missing FD in ancillary data".into(),
                        ));
                    }
                    args.push(WireArg::AncillaryFd);
                    fd_count += 1;
                } else {
                    return Err(WireError::ProtocolError("FD requested but none received".into()));
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

    Ok(args)
}

pub fn encode_message(message: &WaylandMessage) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(8 + message.payload.len());
    out.resize(8, 0);
    LittleEndian::write_u32(&mut out[0..4], message.header.object_id.0);
    let word1 = ((message.header.size as u32) << 16) | (message.header.opcode.0 as u32);
    LittleEndian::write_u32(&mut out[4..8], word1);
    out.extend_from_slice(&message.payload);
    Ok(out)
}

pub fn encode_event(
    object_id: WaylandObjectId,
    opcode: WaylandOpcode,
    args: &[WireArg],
    _registry: &WireObjectRegistry,
) -> Result<WaylandMessage> {
    let mut payload = Vec::with_capacity(estimate_payload_capacity(args));
    for arg in args {
        match arg {
            WireArg::Int(v) => payload.extend_from_slice(&v.to_le_bytes()),
            WireArg::Uint(v) => payload.extend_from_slice(&v.to_le_bytes()),
            WireArg::Fixed(v) => payload.extend_from_slice(&v.to_le_bytes()),
            WireArg::String(s) => encode_string(s, &mut payload),
            WireArg::Object(v) => payload.extend_from_slice(&v.to_le_bytes()),
            WireArg::NewId(v) => payload.extend_from_slice(&v.to_le_bytes()),
            WireArg::Array(a) => encode_array(a, &mut payload),
            WireArg::Fd(_) | WireArg::AncillaryFd => {
                // FD events are not fully supported in this parity check yet,
                // but we can pass a dummy value or handle ancillary.
                // For 'send' event, we just put a dummy uint as per wire spec.
            }
        }
    }
    Ok(WaylandMessage::new(object_id, opcode, payload))
}

fn estimate_payload_capacity(args: &[WireArg]) -> usize {
    args.iter()
        .map(|arg| match arg {
            WireArg::Int(_)
            | WireArg::Uint(_)
            | WireArg::Fixed(_)
            | WireArg::Object(_)
            | WireArg::NewId(_)
            | WireArg::Fd(_)
            | WireArg::AncillaryFd => 4,
            WireArg::String(s) => 4 + align4(s.len() + 1),
            WireArg::Array(a) => 4 + align4(a.len()),
        })
        .sum()
}

const fn align4(value: usize) -> usize {
    (value + 3) & !3
}

#[cfg(test)]
mod tests {
    use super::decode_message;
    use crate::WireError;
    use byteorder::{ByteOrder, LittleEndian};

    #[test]
    fn rejects_invalid_message_size() {
        let mut bytes = vec![0u8; 8];
        LittleEndian::write_u32(&mut bytes[0..4], 1);
        LittleEndian::write_u32(&mut bytes[4..8], (4u32 << 16) | 0);
        assert!(matches!(decode_message(&bytes), Err(WireError::InvalidSize(4))));
    }
}
