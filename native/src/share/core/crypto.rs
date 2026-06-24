use std::io;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub(crate) fn eio<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    let _ = getrandom::getrandom(&mut out);
    out
}

pub(crate) fn random_token(bytes: usize) -> String {
    let mut raw = vec![0u8; bytes];
    let _ = getrandom::getrandom(&mut raw);
    b64(&raw)
}

pub(crate) fn random_hex_token<const N: usize>() -> String {
    hex(&random_bytes::<N>())
}

pub(crate) fn random_uuid_v4() -> String {
    let mut b = random_bytes::<16>();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

pub(crate) fn b64(raw: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(raw)
}

pub(crate) fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD.decode(s.trim()).map_err(|e| e.to_string())
}

pub(crate) fn hex(raw: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(raw.len() * 2);
    for b in raw {
        out.push(H[(b >> 4) as usize] as char);
        out.push(H[(b & 0x0f) as usize] as char);
    }
    out
}

pub(crate) fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("hex length must be even".into());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_val(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err("invalid hex".into()),
    }
}

pub(crate) fn public_fingerprint(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    hex(&digest[..16])
}

pub(crate) fn presence_payload(
    kind: &str,
    relation_id: &str,
    device_id: &str,
    public_key: &str,
    node_id: &str,
    relay_url: &str,
    candidates: &[String],
    expires_at: i64,
    nonce: &str,
) -> String {
    let mut c = candidates.to_vec();
    c.sort();
    format!(
        "{kind}|{relation_id}|{device_id}|{public_key}|{node_id}|{relay_url}|{}|{expires_at}|{nonce}",
        c.join(",")
    )
}

pub(crate) fn hmac_proof(secret: &[u8], payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts arbitrary key length");
    mac.update(payload.as_bytes());
    b64(&mac.finalize().into_bytes())
}

pub(crate) fn verify_hmac(secret: &[u8], payload: &str, proof: &str) -> bool {
    let Ok(expected) = b64_decode(proof) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(payload.as_bytes());
    mac.verify_slice(&expected).is_ok()
}

#[cfg(test)]
pub(crate) fn sanitize_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let n = n.trim().trim_matches('.').to_string();
    if n.is_empty() {
        "datei".to_string()
    } else {
        n
    }
}
