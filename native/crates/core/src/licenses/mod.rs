//! License texts of every Rust crate linked into the shipped programs (plan
//! C4, INT-LIC). `native/rust-licenses.ps1` generates
//! `third-party/rust-crates/THIRD-PARTY-LICENSES.txt` with cargo-about, and
//! `native/build.rs` sets the cfg `se_crate_licenses` only when that file
//! exists. A development build without it embeds nothing, so `se licenses`,
//! the desktop window and the Android page show [`MISSING_NOTICE`]. A release
//! cannot lack the file: `native/publish-release-local.ps1` runs
//! `rust-licenses.ps1 -Mode Check` before its first build.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

/// Shown by every surface when the build embedded no license file.
pub const MISSING_NOTICE: &str = "Lizenzverzeichnis fehlt in diesem Entwicklungsbau: \
     third-party/rust-crates/THIRD-PARTY-LICENSES.txt lag beim Bau nicht vor.";

/// Heading of the crate list in the file (layout of `native/rust-licenses.ps1`).
pub const CRATES_HEADING: &str = "Crates by manifest (name version license)";
/// Heading that follows the crate list.
pub const TEXTS_HEADING: &str = "License texts";

/// The file as `native/build.rs` embeds it: deflated (about a twelfth of
/// its size), inflated once on first use.
#[cfg(se_crate_licenses)]
const EMBEDDED: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("OUT_DIR"),
    "/THIRD-PARTY-LICENSES.txt.deflate"
)));
#[cfg(not(se_crate_licenses))]
const EMBEDDED: Option<&[u8]> = None;

/// One crate line of the file: `name version license-expression`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CrateLicense {
    pub name: String,
    pub version: String,
    pub license: String,
}

/// The embedded file, `None` in a development build without it.
pub fn text() -> Option<&'static str> {
    static TEXT: OnceLock<Option<String>> = OnceLock::new();
    TEXT.get_or_init(|| EMBEDDED.map(inflate_text)).as_deref()
}

/// The text of the deflated file; `native/build.rs` proved its round trip
/// and that it is UTF-8 before embedding it.
fn inflate_text(packed: &[u8]) -> String {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::DeflateDecoder::new(packed)
        .read_to_string(&mut text)
        .expect("native/build.rs proved the embedded license file inflates to UTF-8");
    text
}

/// Every crate of the file once (the union of all manifests), ordered by
/// name and version; empty without an embedded file.
pub fn crates() -> Vec<CrateLicense> {
    static CRATES: OnceLock<Vec<CrateLicense>> = OnceLock::new();
    CRATES
        .get_or_init(|| text().map(parse_crates).unwrap_or_default())
        .clone()
}

/// SHA-256 (lowercase hex) of the embedded file with LF line ends, the form
/// the generator writes, so a CRLF checkout yields the same value.
pub fn sha256() -> Option<String> {
    static DIGEST: OnceLock<Option<String>> = OnceLock::new();
    DIGEST.get_or_init(|| text().map(lf_sha256)).clone()
}

/// Summary shared by `se licenses --json` and the bridge method
/// `app.licenses`: `{"embedded", "sha256", "crates": [{name, version, license}]}`.
pub fn summary_json() -> Value {
    json!({
        "embedded": text().is_some(),
        "sha256": sha256(),
        "crates": crates(),
    })
}

/// SHA-256 of `text` after CRLF → LF.
pub fn lf_sha256(text: &str) -> String {
    let digest = Sha256::digest(text.replace("\r\n", "\n").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Reads the crate lines between [`CRATES_HEADING`] and [`TEXTS_HEADING`].
/// `[manifest]` labels, the `-` underline and blank lines are skipped; a crate
/// that several manifests list appears once.
pub fn parse_crates(text: &str) -> Vec<CrateLicense> {
    let mut lines = text
        .lines()
        .skip_while(|line| line.trim_end() != CRATES_HEADING);
    if lines.next().is_none() {
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    let mut crates = Vec::new();
    for line in lines {
        let line = line.trim_end();
        if line == TEXTS_HEADING {
            break;
        }
        if line.is_empty() || line.starts_with('[') || line.chars().all(|c| c == '-') {
            continue;
        }
        let mut parts = line.splitn(3, ' ');
        let (Some(name), Some(version), Some(license)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if seen.insert((name.to_string(), version.to_string())) {
            crates.push(CrateLicense {
                name: name.to_string(),
                version: version.to_string(),
                license: license.to_string(),
            });
        }
    }
    crates.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| version_order(&a.version, &b.version))
    });
    crates
}

/// Numeric parts of a version compare as numbers, all others as text.
fn version_order(a: &str, b: &str) -> Ordering {
    fn parts(version: &str) -> Vec<Result<u64, &str>> {
        version
            .split(['.', '-', '+'])
            .map(|part| part.parse::<u64>().map_err(|_| part))
            .collect()
    }
    parts(a).cmp(&parts(b))
}
