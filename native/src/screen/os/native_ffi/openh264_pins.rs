//! What this build loads of OpenH264 (decision C2 „OpenH264-Bezug“, NR N8):
//! version 2.6.0 from Cisco's server over HTTPS, the size and SHA-256 of the
//! compressed file and the SHA-256 of the unpacked library (the latter also
//! in the list of `openh264-sys2` 0.9.9), for 64-bit Windows and Linux on
//! x86-64, the targets of the release. Cisco publishes only MD5 sums; the
//! integrity rests on these pins alone, and they change only with a release.
//! The Cisco binary license v1.0 is shown before the download; its text
//! lies word for word in `third-party/openh264/BINARY_LICENSE.txt` and is
//! embedded by `screen::core::licenses` (4 612 bytes, SHA-256 pinned there).

/// One download of the Cisco library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenH264Pin {
    pub url: &'static str,
    pub compressed_size: u64,
    pub compressed_sha256: &'static str,
    /// File name of the unpacked library in the store.
    pub library_name: &'static str,
    pub library_sha256: &'static str,
}

/// The pin of this platform; `None` where Cisco's file does not fit the
/// build (other architectures).
#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) const PIN: Option<OpenH264Pin> = Some(OpenH264Pin {
    url: "https://ciscobinary.openh264.org/openh264-2.6.0-win64.dll.bz2",
    compressed_size: 452_053,
    compressed_sha256: "dab5f2a872777f9a58b69bfa9fbcf20d9f82f2d6ec91383fd70bff49bd34ac9f",
    library_name: "openh264-2.6.0-win64.dll",
    library_sha256: "2076cb5675ec6c1a4c70e7a2a322552f547b6eeed649d6dfcd9e02a543b24691",
});
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) const PIN: Option<OpenH264Pin> = Some(OpenH264Pin {
    url: "https://ciscobinary.openh264.org/libopenh264-2.6.0-linux64.8.so.bz2",
    compressed_size: 634_264,
    compressed_sha256: "27ab53323c110b76214c1c72222f459d17febbcd1e252136cadc292b0308d75b",
    library_name: "libopenh264-2.6.0-linux64.8.so",
    library_sha256: "2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04",
});
#[cfg(not(any(
    all(windows, target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
pub(crate) const PIN: Option<OpenH264Pin> = None;

/// Host of every download; redirects are not followed.
pub(crate) const DOWNLOAD_HOST: &str = "https://ciscobinary.openh264.org/";
/// Largest unpacked library accepted (the 2.6.0 files are about 1 MB).
pub(crate) const MAX_LIBRARY_BYTES: u64 = 16 * 1024 * 1024;

// The Cisco binary license and its pin live in `screen::core::licenses`
// (embedded from `third-party/openh264/BINARY_LICENSE.txt`, W6).
pub(crate) use crate::screen::core::licenses::{CISCO_BINARY_LICENSE, CISCO_BINARY_LICENSE_SHA256};
