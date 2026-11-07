//! Ed25519 signature of a release manifest (E12, review r-lokal-2,
//! 2026-11-07).
//!
//! The release workflow signs `release-manifest` (version, then SHA-256,
//! size and name of every asset) with the private key held as the GitHub
//! Actions secret `SE_UPDATE_SIGNING_KEY`; this program carries the public
//! key. The format is fixed:
//!
//! ```text
//! smart-explorer-release-manifest 1
//! version X.Y.Z
//! <sha256> <size> <name>        (one line per asset, names ordinal ascending)
//! ```
//!
//! UTF-8 without BOM, every line ends with LF, no empty lines, no trailing
//! blanks. `release-manifest.sig` holds the 64 signature bytes as 128 hex
//! characters and one LF. The signed bytes are the whole manifest file.
use std::collections::BTreeMap;

/// Asset name of the manifest beside the release payloads.
pub(super) const MANIFEST_ASSET: &str = "release-manifest";
/// Asset name of the detached signature of [`MANIFEST_ASSET`].
pub(super) const SIGNATURE_ASSET: &str = "release-manifest.sig";
/// Upper bound of a manifest read from a feed (26 assets need about 3 KiB).
pub(super) const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Upper bound of a signature file read from a feed (129 bytes expected).
pub(super) const MAX_SIGNATURE_BYTES: u64 = 1024;

const HEADER: &str = "smart-explorer-release-manifest 1";

/// The project's release key: raw Ed25519 public key (RFC 8032), the
/// counterpart of `SE_UPDATE_SIGNING_KEY`.
const RELEASE_PUBLIC_KEY: [u8; 32] = [
    0x1a, 0x3c, 0xc5, 0xff, 0x34, 0x55, 0x26, 0xa7, 0x1e, 0x56, 0x24, 0x16, 0x5b, 0x47, 0xcf, 0xc2,
    0x7f, 0x76, 0x6a, 0xf7, 0x07, 0x09, 0x3b, 0xfb, 0x6a, 0xed, 0x28, 0xce, 0xfb, 0xa0, 0xcd, 0x6b,
];

/// One signed asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ManifestEntry {
    pub(super) sha256: String,
    pub(super) size: u64,
}

/// A manifest whose signature was verified with the release key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReleaseManifest {
    version: String,
    entries: BTreeMap<String, ManifestEntry>,
}

impl ReleaseManifest {
    pub(super) fn entry(&self, asset: &str) -> Option<&ManifestEntry> {
        self.entries.get(asset)
    }

    /// The signed size of `asset` after checking that the hash its
    /// `.sha256` sidecar names is the signed one. `Ok(None)`: the release
    /// does not contain `asset`.
    pub(super) fn bind(&self, asset: &str, sidecar_sha256: &str) -> Result<Option<u64>, String> {
        let Some(entry) = self.entry(asset) else {
            return Ok(None);
        };
        if !entry.sha256.eq_ignore_ascii_case(sidecar_sha256.trim()) {
            return Err(format!(
                "update_signature_mismatch: {asset} has SHA-256 {sidecar_sha256} in its sidecar, the signed release manifest names {}",
                entry.sha256
            ));
        }
        Ok(Some(entry.size))
    }
}

/// Verifies `signature` (the text of `release-manifest.sig`) over the exact
/// `manifest` bytes with the release key, then parses the manifest and
/// requires its version to be `expected_version`.
pub(super) fn verify_manifest(
    manifest: &[u8],
    signature: &[u8],
    expected_version: &str,
) -> Result<ReleaseManifest, String> {
    let signature = decode_signature(signature)?;
    let key = release_key();
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(manifest, &signature)
        .map_err(|_| {
            "update_signature_invalid: the release manifest's signature does not match the release key"
                .to_string()
        })?;
    let parsed = parse_manifest(manifest)?;
    if parsed.version != expected_version {
        return Err(format!(
            "update_signature_version: the signed release manifest is for version {}, the feed offers {expected_version}",
            parsed.version
        ));
    }
    Ok(parsed)
}

fn decode_signature(raw: &[u8]) -> Result<[u8; 64], String> {
    let text = raw.strip_suffix(b"\n").unwrap_or(raw);
    if text.len() != 128 {
        return Err(format!(
            "update_signature_malformed: {SIGNATURE_ASSET} must hold 128 hex characters, found {} bytes",
            text.len()
        ));
    }
    let mut signature = [0u8; 64];
    for (slot, pair) in signature.iter_mut().zip(text.chunks_exact(2)) {
        let high = hex_value(pair.first().copied());
        let low = hex_value(pair.get(1).copied());
        match (high, low) {
            (Some(high), Some(low)) => *slot = (high << 4) | low,
            _ => {
                return Err(format!(
                    "update_signature_malformed: {SIGNATURE_ASSET} is not hex"
                ))
            }
        }
    }
    Ok(signature)
}

fn hex_value(byte: Option<u8>) -> Option<u8> {
    match byte? {
        digit @ b'0'..=b'9' => Some(digit - b'0'),
        letter @ b'a'..=b'f' => Some(letter - b'a' + 10),
        letter @ b'A'..=b'F' => Some(letter - b'A' + 10),
        _ => None,
    }
}

/// Strict parse of the signed text; any deviation from the format is an
/// error, so two readers can never disagree about one signed file.
pub(super) fn parse_manifest(raw: &[u8]) -> Result<ReleaseManifest, String> {
    let malformed = |detail: &str| format!("update_manifest_malformed: {detail}");
    let text = std::str::from_utf8(raw).map_err(|_| malformed("not UTF-8"))?;
    let body = text
        .strip_suffix('\n')
        .ok_or_else(|| malformed("the last line does not end with LF"))?;
    let mut lines = body.split('\n');
    if lines.next() != Some(HEADER) {
        return Err(malformed("unknown header line"));
    }
    let version = lines
        .next()
        .and_then(|line| line.strip_prefix("version "))
        .filter(|version| valid_version(version))
        .ok_or_else(|| malformed("the second line is not `version X.Y.Z`"))?
        .to_string();
    let mut entries = BTreeMap::new();
    let mut previous: Option<&str> = None;
    for line in lines {
        let mut fields = line.splitn(3, ' ');
        let (Some(sha256), Some(size), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(malformed("an asset line lacks `<sha256> <size> <name>`"));
        };
        let hex = sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        let decimal = !size.is_empty()
            && size.bytes().all(|byte| byte.is_ascii_digit())
            && (size == "0" || !size.starts_with('0'));
        let clean_name = !name.is_empty()
            && !name.ends_with(' ')
            && !name.starts_with(' ')
            && !name.chars().any(char::is_control);
        if !hex || !decimal || !clean_name {
            return Err(malformed(&format!(
                "asset line for {name:?} is not canonical"
            )));
        }
        if previous.is_some_and(|before| before.as_bytes() >= name.as_bytes()) {
            return Err(malformed(
                "asset names are not unique and ordinal ascending",
            ));
        }
        previous = Some(name);
        let size = size
            .parse::<u64>()
            .map_err(|_| malformed(&format!("size of {name:?} out of range")))?;
        entries.insert(
            name.to_string(),
            ManifestEntry {
                sha256: sha256.to_string(),
                size,
            },
        );
    }
    if entries.is_empty() {
        return Err(malformed("no asset lines"));
    }
    Ok(ReleaseManifest { version, entries })
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
}

#[cfg(not(any(test, feature = "test-support")))]
fn release_key() -> [u8; 32] {
    RELEASE_PUBLIC_KEY
}

#[cfg(any(test, feature = "test-support"))]
static TEST_RELEASE_KEY: std::sync::Mutex<Option<[u8; 32]>> = std::sync::Mutex::new(None);

#[cfg(any(test, feature = "test-support"))]
fn release_key() -> [u8; 32] {
    match TEST_RELEASE_KEY.lock() {
        Ok(key) => key.unwrap_or(RELEASE_PUBLIC_KEY),
        Err(poisoned) => poisoned.into_inner().unwrap_or(RELEASE_PUBLIC_KEY),
    }
}

/// Test port: verify with `key` instead of the release key until the guard
/// drops. Process-wide, so tests that use it must not run concurrently with
/// other signature tests.
#[cfg(any(test, feature = "test-support"))]
pub struct TestReleaseKey {
    previous: Option<[u8; 32]>,
}

#[cfg(any(test, feature = "test-support"))]
pub fn use_test_release_key(key: [u8; 32]) -> TestReleaseKey {
    let mut slot = TEST_RELEASE_KEY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    TestReleaseKey {
        previous: slot.replace(key),
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestReleaseKey {
    fn drop(&mut self) {
        let mut slot = TEST_RELEASE_KEY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = self.previous.take();
    }
}

#[cfg(test)]
#[path = "release_signature_tests.rs"]
mod tests;
