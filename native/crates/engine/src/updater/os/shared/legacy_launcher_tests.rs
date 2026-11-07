//! The bridge for callers of the earlier app (E12; suite expectation
//! E12-brücke of the review 2026-11-07): when the release is signed, the
//! three hashes the earlier app staged must be the signed ones. The bridge's
//! entry point needs a staged `se` that proves a managed runtime (a program
//! run by the capability probe), so the check of the signed hashes is tested
//! where it sits, `FeedBoundary::check_signed`.
use super::FeedBoundary;
use crate::updater::feed_trust::read_trust;
use crate::updater::feed_trust::tests::fixture::{
    manifest as manifest_bytes, sha256_hex, AppData, ReleaseFolder, TestKey, VERSION,
};
use crate::updater::feed_trust::FeedTrust;

fn boundary(release: &ReleaseFolder, app: &str, cli: &str, helper: &str) -> FeedBoundary {
    FeedBoundary {
        root: release.path().to_path_buf(),
        source: release.path().to_string_lossy().into_owned(),
        version: VERSION.into(),
        app_sha256: app.into(),
        cli_sha256: cli.into(),
        helper_sha256: helper.into(),
    }
}

fn signed(release: &ReleaseFolder) -> FeedTrust {
    read_trust(&release.feed(), VERSION).expect("the signed release")
}

#[test]
fn lokal_task_bridge_binds_the_staged_hashes_to_the_signed_manifest() {
    let _app = AppData::new();
    let key = TestKey::install();
    let release = ReleaseFolder::new(VERSION);
    release.sign(&key);
    let trust = signed(&release);
    let manifest = trust.manifest().expect("a signed release has a manifest");
    let (app, cli, helper) = (
        release.payload("app").sha256(),
        release.payload("cli").sha256(),
        release.payload("updater").sha256(),
    );
    let other = sha256_hex(b"a payload the earlier app staged from somewhere else");

    boundary(&release, &app, &cli, &helper)
        .check_signed(manifest)
        .expect("the staged hashes are the signed ones");
    boundary(&release, &app.to_uppercase(), &cli, &helper)
        .check_signed(manifest)
        .expect("hex case does not matter");

    for (case, wrong) in [
        ("app", boundary(&release, &other, &cli, &helper)),
        ("cli", boundary(&release, &app, &other, &helper)),
        ("updater", boundary(&release, &app, &cli, &other)),
    ] {
        let error = wrong
            .check_signed(manifest)
            .err()
            .unwrap_or_else(|| panic!("{case}: another hash than the signed one was accepted"));
        assert!(
            error.starts_with("update_signature_mismatch"),
            "{case}: {error}"
        );
    }

    // A signed release that does not list one of the three is no release of
    // this bridge's three payloads.
    let listed: Vec<(&str, &[u8])> = release
        .payloads()
        .iter()
        .filter(|payload| payload.role != "cli")
        .map(|payload| (payload.name, payload.bytes.as_slice()))
        .collect();
    let without_cli = manifest_bytes(VERSION, &listed);
    release.write("release-manifest.sig", &key.sign(&without_cli));
    release.write("release-manifest", &without_cli);
    let trust = signed(&release);
    let error = boundary(&release, &app, &cli, &helper)
        .check_signed(trust.manifest().expect("a signed release has a manifest"))
        .err()
        .expect("a manifest without the cli");
    assert!(
        error.starts_with("legacy_launcher_release_mismatch"),
        "{error}"
    );
}

/// Without a manifest the bridge keeps its earlier sidecar-only check: the
/// release has no signed hashes to compare.
#[test]
fn lokal_task_bridge_has_no_signed_hashes_for_an_unsigned_release() {
    let _app = AppData::new();
    let release = ReleaseFolder::new(VERSION);
    let trust = signed(&release);
    assert!(trust.manifest().is_none());
    assert!(trust.unsigned().is_some());
}
