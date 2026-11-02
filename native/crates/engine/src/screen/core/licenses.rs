//! License texts of the screen strand's building blocks (SCR-M10, W6),
//! embedded with `include_str!`, so the installer, the Linux script, the
//! update feed and the program carry the same texts without feed files of
//! their own. Shown in Bildschirm → Kodierung → Lizenzen and by `se screen
//! codec licenses`.
//!
//! The Cisco binary license of OpenH264 v1.0 lies word for word in
//! `third-party/openh264/BINARY_LICENSE.txt` (read on 2026-10-18 from
//! `https://www.openh264.org/BINARY_LICENSE.txt`, 4 612 bytes, LF line
//! ends); its SHA-256 is pinned here and checked by
//! `screen_task_cisco_license_verbatim`. Condition 4 of that license asks for
//! the complete text wherever the binary is offered: the program shows it
//! before every download.

/// The Cisco binary license of OpenH264, v1.0, word for word.
pub const CISCO_BINARY_LICENSE: &str =
    include_str!("../../../../../../third-party/openh264/BINARY_LICENSE.txt");
/// SHA-256 of `CISCO_BINARY_LICENSE`.
pub const CISCO_BINARY_LICENSE_SHA256: &str =
    "bd9f363c5ea11ef723d0304cddacb5273c43c0e1194097c7a045d05273635418";
/// Length of `CISCO_BINARY_LICENSE` in bytes.
pub const CISCO_BINARY_LICENSE_BYTES: usize = 4_612;

/// One embedded license text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LicenseText {
    /// Short key (`se screen codec licenses --name`).
    pub key: &'static str,
    /// What it covers.
    pub component: &'static str,
    /// The license's name.
    pub license: &'static str,
    pub text: &'static str,
}

/// Every license text of the screen strand this build carries. The notices
/// of `openh264-rs`, `ashpd`, `wide` and `bzip2-rs` follow with their files
/// under `third-party/` (SCR-M10).
pub fn screen_licenses() -> Vec<LicenseText> {
    vec![LicenseText {
        key: "openh264",
        component: "OpenH264 Video Codec provided by Cisco Systems, Inc. (downloaded by the user)",
        license: "Cisco binary license v1.0",
        text: CISCO_BINARY_LICENSE,
    }]
}

/// The license of `key`.
pub fn license(key: &str) -> Option<LicenseText> {
    screen_licenses().into_iter().find(|entry| entry.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn screen_task_cisco_license_verbatim() {
        let text = CISCO_BINARY_LICENSE.as_bytes();
        assert_eq!(text.len(), CISCO_BINARY_LICENSE_BYTES);
        let hash: String = Sha256::digest(text)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hash, CISCO_BINARY_LICENSE_SHA256);
        assert!(hash.starts_with("bd9f363c") && hash.ends_with("5418"));
        // Condition 4 is part of the text; no CR from a checkout slipped in.
        assert!(
            CISCO_BINARY_LICENSE.contains("OpenH264 Video Codec provided by Cisco Systems, Inc.")
        );
        assert!(!CISCO_BINARY_LICENSE.contains('\r'));
        assert_eq!(
            license("openh264").map(|entry| entry.text),
            Some(CISCO_BINARY_LICENSE)
        );
        assert_eq!(license("x264"), None);
    }
}
