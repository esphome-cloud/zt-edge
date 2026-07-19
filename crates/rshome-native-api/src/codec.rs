use bytes::{Buf, BufMut, BytesMut};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

/// Maximum accepted frame payload size (64 KiB).
///
/// Frames reporting a payload larger than this are rejected with `InvalidData`
/// so that a misbehaving device cannot force the client to allocate unbounded memory.
pub const MAX_FRAME_PAYLOAD: u64 = 64 * 1024;

/// Maximum number of bytes a valid varint can occupy.
///
/// A u64 needs at most 10 bytes in varint encoding (⌈64/7⌉ = 10). Any varint
/// that exceeds this length is considered malicious and rejected.
pub const MAX_VARINT_BYTES: usize = 10;

/// ESPHome frame format: `0x00 | varint(payload_len) | varint(msg_type) | payload_bytes`
pub struct EspHomeCodec;

/// Read a varint from `src` without consuming bytes. Returns `(value, bytes_consumed)`.
///
/// Returns `None` if the varint is incomplete (need more bytes) or if it exceeds
/// `MAX_VARINT_BYTES` (anti-DoS: rejects pathologically long varint sequences).
pub fn peek_varint(src: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    for (i, &byte) in src.iter().enumerate() {
        if i >= MAX_VARINT_BYTES {
            return None; // exceeds maximum varint length — reject
        }
        result |= u64::from(byte & 0x7F) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        if shift >= 64 {
            return None; // overflow
        }
    }
    None // incomplete varint — need more bytes
}

/// Write a varint into `dst`.
fn write_varint(value: u64, dst: &mut BytesMut) {
    let mut v = value;
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            dst.put_u8(byte);
            break;
        }
        dst.put_u8(byte | 0x80);
    }
}

impl Decoder for EspHomeCodec {
    type Item = (u32, Vec<u8>); // (msg_type, payload)
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }

        // Check preamble byte
        if src[0] != 0x00 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid ESPHome preamble: {:#04x}", src[0]),
            ));
        }

        if src.len() < 2 {
            return Ok(None);
        }

        // Peek payload length varint (after the 0x00 preamble)
        let (payload_len, len_bytes) = match peek_varint(&src[1..]) {
            Some(v) => v,
            None => return Ok(None), // incomplete
        };

        // Reject oversized payloads before allocating any buffer.
        if payload_len > MAX_FRAME_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame payload too large: {payload_len} bytes (max {MAX_FRAME_PAYLOAD})"),
            ));
        }

        let type_offset = 1 + len_bytes;
        if src.len() <= type_offset {
            return Ok(None);
        }

        // Peek msg_type varint
        let (msg_type, type_bytes) = match peek_varint(&src[type_offset..]) {
            Some(v) => v,
            None => return Ok(None), // incomplete
        };

        let header_len = 1 + len_bytes + type_bytes;
        let total_len = header_len + payload_len as usize;

        if src.len() < total_len {
            return Ok(None); // payload not yet fully arrived
        }

        // Consume header
        src.advance(header_len);

        // Copy payload
        let payload = src[..payload_len as usize].to_vec();
        src.advance(payload_len as usize);

        Ok(Some((msg_type as u32, payload)))
    }
}

impl Encoder<(u32, Vec<u8>)> for EspHomeCodec {
    type Error = io::Error;

    fn encode(&mut self, item: (u32, Vec<u8>), dst: &mut BytesMut) -> Result<(), Self::Error> {
        let (msg_type, payload) = item;
        dst.put_u8(0x00);
        write_varint(payload.len() as u64, dst);
        write_varint(u64::from(msg_type), dst);
        dst.put_slice(&payload);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::codec::{Decoder, Encoder};

    fn encode_frame(msg_type: u32, payload: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        EspHomeCodec
            .encode((msg_type, payload.to_vec()), &mut buf)
            .unwrap();
        buf
    }

    #[test]
    fn roundtrip_simple() {
        let payload = b"hello world";
        let mut buf = encode_frame(1, payload);
        let result = EspHomeCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.0, 1);
        assert_eq!(result.1, payload);
    }

    #[test]
    fn roundtrip_zero_payload() {
        let mut buf = encode_frame(8, &[]);
        let result = EspHomeCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.0, 8);
        assert!(result.1.is_empty());
    }

    #[test]
    fn partial_frame_returns_none() {
        let mut buf = encode_frame(1, b"hello");
        // Truncate to partial
        let full_len = buf.len();
        let partial = buf.split_to(full_len - 2);
        let mut partial = partial;
        let result = EspHomeCodec.decode(&mut partial).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn multi_frame_buffer() {
        let mut buf = BytesMut::new();
        EspHomeCodec
            .encode((1, b"first".to_vec()), &mut buf)
            .unwrap();
        EspHomeCodec
            .encode((2, b"second".to_vec()), &mut buf)
            .unwrap();

        let first = EspHomeCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(first.0, 1);
        assert_eq!(first.1, b"first");

        let second = EspHomeCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(second.0, 2);
        assert_eq!(second.1, b"second");
    }

    #[test]
    fn invalid_preamble_returns_error() {
        let mut buf = BytesMut::from(&[0x01u8, 0x00, 0x01][..]);
        let result = EspHomeCodec.decode(&mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn varint_one_byte() {
        // msg_type 7 (< 128) → single-byte varint
        let mut buf = encode_frame(7, b"ping");
        let result = EspHomeCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.0, 7);
    }

    #[test]
    fn varint_two_byte() {
        // msg_type 128 → two-byte varint: 0x80 0x01
        let mut buf = encode_frame(128, b"data");
        let result = EspHomeCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.0, 128);
    }

    #[test]
    fn varint_large_payload() {
        // payload_len > 127 → multi-byte varint for len
        let payload = vec![0xABu8; 200];
        let mut buf = encode_frame(25, &payload);
        let result = EspHomeCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.0, 25);
        assert_eq!(result.1.len(), 200);
    }

    #[test]
    fn oversized_payload_returns_error() {
        // Craft a frame that claims payload_len = MAX_FRAME_PAYLOAD + 1
        // (we only need the header — the decoder rejects before reading the body)
        let too_large = MAX_FRAME_PAYLOAD + 1;
        let mut buf = BytesMut::new();
        buf.put_u8(0x00); // preamble
                          // write varint for too_large
        let mut v = too_large;
        loop {
            let byte = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                buf.put_u8(byte);
                break;
            }
            buf.put_u8(byte | 0x80);
        }
        // msg_type varint (1 byte, value = 1)
        buf.put_u8(0x01);
        // No actual payload bytes needed — rejection happens before we try to read them

        let result = EspHomeCodec.decode(&mut buf);
        assert!(result.is_err(), "oversized payload must return an error");
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
