//! What the update check does with a release that carries no valid signature
//! (E12, r-lokal-3; suite expectation E12-http of the review 2026-11-07):
//! the check asks for the user's consent before any payload is fetched, the
//! consent covers one version, and a bad signature is refused even with it.
//! Feeds are folders and a loopback HTTP server; nothing leaves the machine.
use super::{check_and_stage, UnsignedConsent, UpdateMsg};
use crate::updater::download_bound::tests::http_fixture::{HttpFeed, Route};
use crate::updater::feed_trust::tests::fixture::{
    sign_with_another_key, AppData, ReleaseFolder, TestKey, VERSION,
};
use crate::updater::feed_trust::UnsignedFeed;
use crate::updater::staging::stage_from_feed;

const DECISION_FILES: [&str; 3] = ["version.txt", "release-manifest", "release-manifest.sig"];

fn assert_needs_consent(
    result: Result<Option<UpdateMsg>, String>,
    expected: UnsignedFeed,
    case: &str,
) {
    match result {
        Ok(Some(UpdateMsg::UnsignedConsentNeeded { version, feed })) => {
            assert_eq!(version, VERSION, "{case}");
            assert_eq!(feed, expected, "{case}");
        }
        other => panic!("{case}: expected the consent question, got {other:?}"),
    }
}

/// E12-http: an `http://` feed without a manifest asks first; the question
/// costs only the three small files; consent for this version stages the
/// bundle and records why it is unsigned; consent for another version asks
/// again.
#[test]
fn lokal_task_plain_http_feed_without_manifest_asks_before_any_payload() {
    let app = AppData::new();
    let release = ReleaseFolder::new(VERSION);
    let server = HttpFeed::start();
    server.serve_folder(&release);
    crate::updater::set_update_source(&server.base()).expect("the update source is stored");

    assert_needs_consent(
        check_and_stage(true, &UnsignedConsent::none()),
        UnsignedFeed::PlainHttp,
        "without consent",
    );
    let error = stage_from_feed(&server.feed(), VERSION, &UnsignedConsent::none())
        .err()
        .expect("staging without consent");
    assert!(error.starts_with("unsigned_http_feed"), "{error}");
    assert!(error.contains("unsigned http feed"), "{error}");
    assert!(error.contains("--allow-unsigned"), "{error}");
    let requests = server.requests();
    assert!(
        requests
            .iter()
            .all(|path| DECISION_FILES.contains(&path.as_str())),
        "a payload was requested before consent: {requests:?}"
    );
    assert_eq!(app.staged_names(), Vec::<String>::new());

    assert_needs_consent(
        check_and_stage(true, &UnsignedConsent::for_version("999.0.1")),
        UnsignedFeed::PlainHttp,
        "consent for another version",
    );
    assert_eq!(app.staged_names(), Vec::<String>::new());

    match check_and_stage(true, &UnsignedConsent::for_version(VERSION)) {
        Ok(Some(UpdateMsg::Staged(bundle))) => {
            assert_eq!(bundle.version(), VERSION);
            assert_eq!(bundle.unsigned(), Some(UnsignedFeed::PlainHttp));
            assert_eq!(bundle.app().sha256(), release.payload("app").sha256());
        }
        other => panic!("consent for this version did not stage: {other:?}"),
    }
}

/// E12-http: a folder feed without manifest is the user's own feed: staged
/// without a question, noted as unsigned.
#[test]
fn lokal_task_folder_feed_without_manifest_is_staged_and_noted() {
    let _app = AppData::new();
    let release = ReleaseFolder::new(VERSION);
    crate::updater::set_update_source(&release.path().to_string_lossy())
        .expect("the update source is stored");
    match check_and_stage(true, &UnsignedConsent::none()) {
        Ok(Some(UpdateMsg::Staged(bundle))) => {
            assert_eq!(bundle.version(), VERSION);
            assert_eq!(bundle.unsigned(), Some(UnsignedFeed::Custom));
        }
        other => panic!("a folder feed without manifest: {other:?}"),
    }
}

/// A signed release over HTTP is staged without a question; a signature that
/// does not verify is refused with the consent for exactly this version.
#[test]
fn lokal_task_signed_release_needs_no_question_and_a_bad_signature_no_consent_helps() {
    let app = AppData::new();
    let key = TestKey::install();
    let release = ReleaseFolder::new(VERSION);
    release.sign(&key);
    let server = HttpFeed::start();
    server.serve_folder(&release);
    crate::updater::set_update_source(&server.base()).expect("the update source is stored");

    match check_and_stage(true, &UnsignedConsent::none()) {
        Ok(Some(UpdateMsg::Staged(bundle))) => {
            assert_eq!(bundle.unsigned(), None);
            crate::updater::discard_staged_update(&bundle).expect("the staged update is removed");
        }
        other => panic!("a signed release: {other:?}"),
    }
    assert_eq!(app.staged_names(), Vec::<String>::new());

    let manifest = release.read("release-manifest");
    server.serve(
        "release-manifest.sig",
        Route::plain(sign_with_another_key(&manifest)),
    );
    for (case, consent) in [
        ("without consent", UnsignedConsent::none()),
        ("with consent", UnsignedConsent::for_version(VERSION)),
    ] {
        let error = check_and_stage(true, &consent)
            .err()
            .unwrap_or_else(|| panic!("{case}: a release signed by another key was staged"));
        assert!(
            error.starts_with("update_signature_invalid"),
            "{case}: {error}"
        );
        assert_eq!(app.staged_names(), Vec::<String>::new(), "{case}");
    }
}
