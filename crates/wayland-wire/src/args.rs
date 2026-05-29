use crate::{Result, WireError};
use byteorder::{ByteOrder, LittleEndian};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FakeFd(pub u32);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WireArg {
    Int(i32),
    Uint(u32),
    Fixed(i32), // 24.8 fixed point
    String(String),
    Object(u32),
    NewId(u32),
    Array(Vec<u8>),
    Fd(FakeFd),
    AncillaryFd, // Represents an FD to be consumed from the queue
}

pub fn decode_string(bytes: &[u8], offset: &mut usize) -> Result<String> {
    if bytes.len() < *offset + 4 {
        return Err(WireError::Incomplete);
    }
    let len = LittleEndian::read_u32(&bytes[*offset..*offset + 4]) as usize;
    *offset += 4;

    if len == 0 {
        return Ok("".into());
    }

    if bytes.len() < *offset + len {
        return Err(WireError::Incomplete);
    }

    // Wayland strings are null-terminated. length includes null byte.
    let s_bytes = &bytes[*offset..*offset + len - 1];
    let s = String::from_utf8_lossy(s_bytes).into_owned();

    // 4-byte alignment
    *offset += (len + 3) & !3;

    Ok(s)
}

pub fn encode_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    let len = (bytes.len() + 1) as u32; // +1 for null terminator
    let mut header = [0u8; 4];
    LittleEndian::write_u32(&mut header, len);
    out.extend_from_slice(&header);
    out.extend_from_slice(bytes);
    out.push(0); // null terminator

    // Padding
    let padded_len = (len as usize + 3) & !3;
    let padding = padded_len - len as usize;
    for _ in 0..padding {
        out.push(0);
    }
}

pub fn decode_array(bytes: &[u8], offset: &mut usize) -> Result<Vec<u8>> {
    if bytes.len() < *offset + 4 {
        return Err(WireError::Incomplete);
    }
    let len = LittleEndian::read_u32(&bytes[*offset..*offset + 4]) as usize;
    *offset += 4;

    if bytes.len() < *offset + len {
        return Err(WireError::Incomplete);
    }

    let data = bytes[*offset..*offset + len].to_vec();
    *offset += (len + 3) & !3;

    Ok(data)
}

pub fn encode_array(data: &[u8], out: &mut Vec<u8>) {
    let len = data.len() as u32;
    let mut header = [0u8; 4];
    LittleEndian::write_u32(&mut header, len);
    out.extend_from_slice(&header);
    out.extend_from_slice(data);

    // Padding
    let padded_len = (len as usize + 3) & !3;
    let padding = padded_len - len as usize;
    for _ in 0..padding {
        out.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_alignment() {
        let mut out = Vec::new();
        encode_string("abc", &mut out);
        assert_eq!(out.len(), 8); // 4 (len) + 4 (abc\0)
        assert_eq!(out[4..8], [b'a', b'b', b'c', 0]);

        let mut offset = 0;
        let s = decode_string(&out, &mut offset).expect("decode");
        assert_eq!(s, "abc");
        assert_eq!(offset, 8);
    }

    #[test]
    fn test_array_alignment() {
        let mut out = Vec::new();
        let data = vec![1, 2];
        encode_array(&data, &mut out);
        assert_eq!(out.len(), 8); // 4 (len) + 2 (data) + 2 (padding)

        let mut offset = 0;
        let d = decode_array(&out, &mut offset).expect("decode");
        assert_eq!(d, data);
        assert_eq!(offset, 8);
    }
}
