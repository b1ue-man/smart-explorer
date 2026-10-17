//! The two cipher combinations of format 8 (CryptoLib `v2` = `SIV_GCM`, `v1`
//! = `SIV_CTRMAC`), their fixed sizes and the size formulas of CryptoLib's
//! `FileContentCryptor` (header not included in `ciphertext_size`).
use super::FormatError;

/// Cleartext bytes per content chunk.
pub(crate) const CHUNK_PAYLOAD: usize = 32 * 1024;
/// Length of the encrypted header payload: 8 reserved bytes ‖ content key.
pub(crate) const HEADER_PAYLOAD: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CipherCombo {
    /// Names AES-SIV, contents AES-256-GCM (default of new vaults).
    SivGcm,
    /// Names AES-SIV, contents AES-256-CTR with HMAC-SHA256.
    SivCtrMac,
}

impl CipherCombo {
    /// The `cipherCombo` claim of `vault.cryptomator`.
    pub(crate) fn parse(claim: &str) -> Option<Self> {
        match claim {
            "SIV_GCM" => Some(Self::SivGcm),
            "SIV_CTRMAC" => Some(Self::SivCtrMac),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SivGcm => "SIV_GCM",
            Self::SivCtrMac => "SIV_CTRMAC",
        }
    }

    /// Nonce of headers and chunks: 12 bytes (GCM), 16 bytes (CTR IV).
    pub(crate) fn nonce_len(self) -> usize {
        match self {
            Self::SivGcm => 12,
            Self::SivCtrMac => 16,
        }
    }

    /// Tag of headers and chunks: GCM tag 16 bytes, HMAC-SHA256 32 bytes.
    pub(crate) fn tag_len(self) -> usize {
        match self {
            Self::SivGcm => 16,
            Self::SivCtrMac => 32,
        }
    }

    /// 68 bytes (GCM) or 88 bytes (CTRMAC).
    pub(crate) fn header_size(self) -> usize {
        self.nonce_len() + HEADER_PAYLOAD + self.tag_len()
    }

    /// Nonce and tag around each chunk: 28 bytes (GCM) or 48 bytes (CTRMAC).
    pub(crate) fn chunk_overhead(self) -> usize {
        self.nonce_len() + self.tag_len()
    }

    /// A full chunk: 32 796 bytes (GCM) or 32 816 bytes (CTRMAC).
    pub(crate) fn chunk_size(self) -> usize {
        CHUNK_PAYLOAD + self.chunk_overhead()
    }
}

/// Ciphertext bytes after the header for `cleartext` bytes.
pub(crate) fn ciphertext_size(cleartext: u64, combo: CipherCombo) -> u64 {
    let payload = CHUNK_PAYLOAD as u64;
    let full = cleartext / payload;
    let rest = cleartext % payload;
    let partial = if rest == 0 {
        0
    } else {
        rest + combo.chunk_overhead() as u64
    };
    full.saturating_mul(combo.chunk_size() as u64)
        .saturating_add(partial)
}

/// Cleartext bytes of `ciphertext` bytes after the header. A last chunk that
/// is shorter than its nonce and tag is an error (CryptoLib); a last chunk of
/// exactly nonce and tag is the empty chunk CryptoLib's stream writer ends
/// every file with (C1.7, W3) and counts as zero bytes.
pub(crate) fn cleartext_size(ciphertext: u64, combo: CipherCombo) -> Result<u64, FormatError> {
    let chunk = combo.chunk_size() as u64;
    let overhead = combo.chunk_overhead() as u64;
    let full = ciphertext / chunk;
    let rest = ciphertext % chunk;
    let partial = match rest {
        0 => 0,
        rest if rest < overhead => return Err(FormatError::InvalidSize),
        rest => rest - overhead,
    };
    Ok(full * CHUNK_PAYLOAD as u64 + partial)
}

/// Size of the whole encrypted file (header included) for `cleartext` bytes.
pub(crate) fn file_size(cleartext: u64, combo: CipherCombo) -> u64 {
    ciphertext_size(cleartext, combo).saturating_add(combo.header_size() as u64)
}

/// Cleartext size of a whole encrypted file of `size` bytes (header
/// included); a file shorter than its header is an error.
pub(crate) fn cleartext_file_size(size: u64, combo: CipherCombo) -> Result<u64, FormatError> {
    let body = size
        .checked_sub(combo.header_size() as u64)
        .ok_or(FormatError::InvalidSize)?;
    cleartext_size(body, combo)
}
