//! Which updates may be staged (E12, review r-lokal-2 and r-lokal-3,
//! 2026-11-07).
//!
//! A feed release that carries `release-manifest` and `release-manifest.sig`
//! is staged only when the signature verifies with the release key; every
//! payload must then match the signed hash and size. Without both files:
//!
//! * the project's GitHub releases (`b1ue-man/smart-explorer`, the feed the
//!   installers configure) are older, unsigned releases and need the user's
//!   explicit consent for this one update;
//! * a plain `http://` feed needs the same consent, because anyone on the
//!   path could have replaced program and checksum;
//! * another feed (a folder, an own `https://` server) stays usable and is
//!   reported as unsigned.
//!
//! An invalid signature, a manifest without signature or a signature
//! without manifest is never accepted, with or without consent.
use std::io::Read;

use serde::{Deserialize, Serialize};

use super::feed::{http_get_bytes_optional_404, Feed};
use super::release_signature::{
    verify_manifest, ReleaseManifest, MANIFEST_ASSET, MAX_MANIFEST_BYTES, MAX_SIGNATURE_BYTES,
    SIGNATURE_ASSET,
};

/// Why an update has no valid release signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsignedFeed {
    /// A release of the project's GitHub feed without manifest (versions
    /// published before signing existed).
    OfficialRelease,
    /// A plain `http://` feed without manifest.
    PlainHttp,
    /// Another feed (folder, own `https://` server) without manifest.
    Custom,
}

impl UnsignedFeed {
    /// Whether staging needs the user's consent for this one update.
    pub fn needs_consent(self) -> bool {
        matches!(self, Self::OfficialRelease | Self::PlainHttp)
    }

    /// Stable code for `se update --json` and error texts.
    pub fn code(self) -> &'static str {
        match self {
            Self::OfficialRelease => "unsigned_release",
            Self::PlainHttp => "unsigned_http_feed",
            Self::Custom => "unsigned_feed",
        }
    }

    /// What the user is asked to confirm (desktop dialog, terminal hint).
    pub fn explanation(self) -> &'static str {
        match self {
            Self::OfficialRelease => {
                "Diese Version wurde vor der Release-Signatur veröffentlicht; ob ihre Dateien unverändert sind, lässt sich nicht prüfen."
            }
            Self::PlainHttp => {
                "unsigned http feed: Der Feed ist unverschlüsselt (http://) und nicht signiert; jeder auf dem Netzweg könnte Programm und Prüfsumme ersetzt haben."
            }
            Self::Custom => "Der Feed ist nicht signiert; geprüft wird nur die Prüfsumme des Feeds.",
        }
    }
}

/// The user's explicit consent to stage exactly one unsigned update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnsignedConsent {
    version: Option<String>,
}

impl UnsignedConsent {
    /// No consent: unsigned releases and `http://` feeds are refused.
    pub fn none() -> Self {
        Self::default()
    }

    /// Consent for `version` only; another version asks again.
    pub fn for_version(version: &str) -> Self {
        Self {
            version: Some(version.to_string()),
        }
    }

    fn allows(&self, version: &str) -> bool {
        self.version.as_deref() == Some(version)
    }
}

/// The verified state of one feed release.
#[derive(Clone, Debug)]
pub(super) enum FeedTrust {
    Signed(ReleaseManifest),
    Unsigned(UnsignedFeed),
}

impl FeedTrust {
    pub(super) fn manifest(&self) -> Option<&ReleaseManifest> {
        match self {
            Self::Signed(manifest) => Some(manifest),
            Self::Unsigned(_) => None,
        }
    }

    pub(super) fn unsigned(&self) -> Option<UnsignedFeed> {
        match self {
            Self::Signed(_) => None,
            Self::Unsigned(kind) => Some(*kind),
        }
    }

    /// The consent this update still needs, `None` when it may be staged.
    pub(super) fn missing_consent(
        &self,
        version: &str,
        consent: &UnsignedConsent,
    ) -> Option<UnsignedFeed> {
        self.unsigned()
            .filter(|kind| kind.needs_consent() && !consent.allows(version))
    }
}

/// Reads and verifies the signature of the release `version` in `feed`
/// (already pinned to that version).
pub(super) fn read_trust(feed: &Feed, version: &str) -> Result<FeedTrust, String> {
    let manifest = read_optional(feed, MANIFEST_ASSET, MAX_MANIFEST_BYTES)?;
    let signature = read_optional(feed, SIGNATURE_ASSET, MAX_SIGNATURE_BYTES)?;
    match (manifest, signature) {
        (Some(manifest), Some(signature)) => {
            verify_manifest(&manifest, &signature, version).map(FeedTrust::Signed)
        }
        (Some(_), None) => Err(format!(
            "update_signature_missing: {} carries {MANIFEST_ASSET} without {SIGNATURE_ASSET}",
            feed.display()
        )),
        (None, Some(_)) => Err(format!(
            "update_signature_missing: {} carries {SIGNATURE_ASSET} without {MANIFEST_ASSET}",
            feed.display()
        )),
        (None, None) => Ok(FeedTrust::Unsigned(unsigned_kind(feed))),
    }
}

/// [`read_trust`] plus the consent rule: the error names how to consent.
pub(super) fn admitted_trust(
    feed: &Feed,
    version: &str,
    consent: &UnsignedConsent,
) -> Result<FeedTrust, String> {
    let trust = read_trust(feed, version)?;
    match trust.missing_consent(version, consent) {
        Some(kind) => Err(consent_error(kind, version, feed)),
        None => Ok(trust),
    }
}

/// The refusal of an unsigned update without consent.
pub(super) fn consent_error(kind: UnsignedFeed, version: &str, feed: &Feed) -> String {
    format!(
        "{}: Update v{version} aus {} ist nicht signiert. {} Installation nur nach ausdrücklicher Bestätigung für dieses Update (Desktop: Dialog „Unsigniertes Update“, Terminal: se update --allow-unsigned).",
        kind.code(),
        feed.display(),
        kind.explanation()
    )
}

/// Whether `feed` is a release of the project's own GitHub repository, the
/// feed the installers configure.
pub(super) fn is_official_release_feed(feed: &Feed) -> bool {
    match feed {
        Feed::Http(base) => base
            .to_ascii_lowercase()
            .starts_with("https://github.com/b1ue-man/smart-explorer/releases/"),
        Feed::Local(_) => false,
    }
}

fn unsigned_kind(feed: &Feed) -> UnsignedFeed {
    if is_official_release_feed(feed) {
        return UnsignedFeed::OfficialRelease;
    }
    match feed {
        Feed::Http(base) if base.to_ascii_lowercase().starts_with("http://") => {
            UnsignedFeed::PlainHttp
        }
        _ => UnsignedFeed::Custom,
    }
}

fn read_optional(feed: &Feed, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let bytes = match feed {
        Feed::Local(dir) => {
            let path = dir.join(name);
            let file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(format!("{}: {error}", path.display())),
            };
            let mut bytes = Vec::new();
            file.take(limit.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            bytes
        }
        Feed::Http(base) => {
            match http_get_bytes_optional_404(&format!("{base}/{name}"), limit.saturating_add(1))? {
                Some(bytes) => bytes,
                None => return Ok(None),
            }
        }
    };
    if bytes.len() as u64 > limit {
        return Err(format!(
            "update_signature_malformed: {name} in {} is larger than {limit} bytes",
            feed.display()
        ));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
#[path = "feed_trust_tests.rs"]
pub(in crate::updater) mod tests;
