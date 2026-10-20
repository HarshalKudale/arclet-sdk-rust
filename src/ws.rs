//! Minimal RFC 6455 client framing over plain TCP (no TLS), JSON text frames only.
//! Client frames MUST be masked; server frames are not.

use serde_json::Value;

fn rand_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    let _ = getrandom::getrandom(&mut b);
    b
}

/// Build one masked text frame carrying `text`.
pub fn encode_text(text: &str) -> Vec<u8> {
    encode(0x1, text.as_bytes())
}

fn encode(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let n = payload.len();
    let mask = rand_bytes::<4>();
    let mut out = vec![0x80 | opcode];
    if n < 126 {
        out.push(0x80 | n as u8);
    } else if n < 65536 {
        out.push(0x80 | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

pub fn encode_pong(payload: &[u8]) -> Vec<u8> {
    encode(0xA, payload)
}

pub fn handshake_request(host: &str, port: u16) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let key_bytes = rand_bytes::<16>();
    let mut key = String::new();
    for c in key_bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() { key.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char) } else { key.push('=') }
        }
    }
    format!(
        "GET / HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
}

pub enum Frame {
    Json(Value),
    Ping(Vec<u8>),
    Close,
}

/// Pop as many complete frames as possible off the front of `buf`.
/// Binary / pong / malformed-JSON frames are skipped.
pub fn parse_frames(buf: &mut Vec<u8>) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut pos = 0;
    while buf.len() - pos >= 2 {
        let b = &buf[pos..];
        let opcode = b[0] & 0x0f;
        let masked = b[1] & 0x80 != 0;
        let mut len = (b[1] & 0x7f) as usize;
        let mut off = 2;
        if len == 126 {
            if b.len() < 4 { break; }
            len = u16::from_be_bytes([b[2], b[3]]) as usize;
            off = 4;
        } else if len == 127 {
            if b.len() < 10 { break; }
            len = u64::from_be_bytes(b[2..10].try_into().unwrap()) as usize;
            off = 10;
        }
        let mask = if masked {
            if b.len() < off + 4 { break; }
            off += 4;
            Some([b[off - 4], b[off - 3], b[off - 2], b[off - 1]])
        } else { None };
        if b.len() < off + len { break; }
        let mut payload = b[off..off + len].to_vec();
        if let Some(m) = mask {
            for (i, x) in payload.iter_mut().enumerate() { *x ^= m[i % 4]; }
        }
        pos += off + len;
        match opcode {
            0x8 => frames.push(Frame::Close),
            0x9 => frames.push(Frame::Ping(payload)),
            0x1 => {
                if let Ok(v) = serde_json::from_slice(&payload) { frames.push(Frame::Json(v)); }
            }
            _ => {}
        }
    }
    buf.drain(..pos);
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_masked_frame() {
        for n in [5usize, 200, 70000] {
            let text = format!("{{\"a\":\"{}\"}}", "x".repeat(n));
            let mut buf = encode_text(&text);
            let frames = parse_frames(&mut buf);
            assert!(buf.is_empty());
            match &frames[0] { Frame::Json(v) => assert_eq!(v["a"].as_str().unwrap().len(), n), _ => panic!() }
        }
    }

    #[test]
    fn partial_frame_waits() {
        let full = encode_text("{\"a\":1}");
        let mut buf = full[..full.len() - 1].to_vec();
        assert!(parse_frames(&mut buf).is_empty());
        assert_eq!(buf.len(), full.len() - 1);
    }
}
