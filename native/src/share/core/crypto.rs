use std::io;

pub(crate) fn eio<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

/// Derive the 32-byte Noise PSK from the human code via HKDF-SHA256.
pub(crate) fn psk_from_code(code: &str) -> [u8; 32] {
    let norm = code.trim().to_uppercase();
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(b"smart-explorer-share-v1"), norm.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"psk", &mut okm).expect("32 bytes is a valid HKDF length");
    okm
}

/// A user-presentable random pairing/room code: 8 Crockford-base32 chars (~40
/// bits), unambiguous (no I/L/O/U).
pub fn gen_code() -> String {
    const A: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut raw = [0u8; 8];
    let _ = getrandom::getrandom(&mut raw);
    raw.iter().map(|b| A[(*b as usize) % A.len()] as char).collect()
}

pub(crate) fn sanitize_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let n = n.trim().trim_matches('.').to_string();
    if n.is_empty() {
        "datei".to_string()
    } else {
        n
    }
}
