//! Review B1: a Linux host reports the SHA-1 thumbprint mstsc needs
//! (`CertHash`, design 2.7.2) next to the SHA-256 pin, both from the one
//! certificate file the probe reads.
use super::{certificate, fingerprint_digest, service};
use crate::rdp::core::{CertificateSha1, HostKind, HostProbe, SessionMode, RDP_PROBE_VERSION_V2};
use sha1::Digest as _;

#[test]
fn einrichtung_task_review_linux_host_sends_the_sha1_thumbprint() {
    let pair = rcgen::KeyPair::generate().expect("key");
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("params");
    let issued = params.self_signed(&pair).expect("certificate");
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("rdp.crt");
    std::fs::write(&path, issued.pem()).expect("certificate file");

    let mut found = service("gnome-console", HostKind::Gnome, SessionMode::SharedConsole);
    certificate(&mut found, &path);
    assert!(found.certificate_sha256.is_some(), "{:?}", found.evidence);
    let expected = sha1::Sha1::digest(issued.der().as_ref());
    let thumbprint = found.certificate_sha1.clone().expect("SHA-1 thumbprint");
    assert_eq!(thumbprint.bytes().as_slice(), expected.as_slice());

    // The v2 answer carries it; the v1 answer of an old `RdpProbe` drops it.
    let probe = HostProbe {
        version: RDP_PROBE_VERSION_V2,
        platform: "linux".into(),
        services: vec![found],
        unavailable_reason: None,
        diagnostics: Vec::new(),
    };
    probe.validate().expect("valid v2 probe");
    let old = probe.into_v1();
    old.validate().expect("valid v1 probe");
    assert!(old.services[0].certificate_sha1.is_none());
}

#[test]
fn einrichtung_task_review_openssl_sha1_output_parses() {
    let line = "SHA1 Fingerprint=0A:1B:2C:3D:4E:5F:60:71:82:93:A4:B5:C6:D7:E8:F9:00:11:22:33\n";
    let digest = fingerprint_digest(line).expect("digest");
    let parsed = CertificateSha1::parse(digest).expect("thumbprint");
    assert_eq!(parsed.bytes()[0], 0x0A);
    assert!(fingerprint_digest("no fingerprint").is_err());
}
