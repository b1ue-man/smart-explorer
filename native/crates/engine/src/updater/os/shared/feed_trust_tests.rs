//! Which feeds are unsigned and what each needs (E12; suite expectation
//! E12-einstufung), and the fixtures the other updater tests of the signature
//! share: a loopback HTTP feed, a feed folder, a test signing key and an
//! isolated app data folder.
use super::{
    consent_error, is_official_release_feed, read_trust, unsigned_kind, FeedTrust, UnsignedConsent,
    UnsignedFeed,
};
use crate::updater::download_bound::tests::http_fixture::{HttpFeed, Route};
use crate::updater::feed::Feed;
use fixture::{AppData, ReleaseFolder, VERSION};

/// Shared test fixtures of the updater's signature tests. Everything that is
/// process-wide (the app data folder, the test release key) is taken under
/// one lock held by [`AppData`], so these tests never overlap.
#[allow(dead_code)]
pub(in crate::updater) mod fixture {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use ring::signature::{Ed25519KeyPair, KeyPair};
    use sha2::{Digest, Sha256};

    use crate::updater::feed::Feed;
    use crate::updater::os;

    /// Newer than every build, so the feed always offers an update.
    pub(in crate::updater) const VERSION: &str = "999.0.0";

    static ENVIRONMENT: Mutex<()> = Mutex::new(());

    pub(in crate::updater) fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    pub(in crate::updater) fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The app data folder of this program in a new directory, for one test.
    pub(in crate::updater) struct AppData {
        _lock: MutexGuard<'static, ()>,
        _root: tempfile::TempDir,
        saved: (&'static str, Option<OsString>),
    }

    impl AppData {
        pub(in crate::updater) fn new() -> Self {
            let lock = ENVIRONMENT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let root = tempfile::tempdir().expect("a temporary app data root");
            let key = if cfg!(windows) {
                "APPDATA"
            } else {
                "XDG_DATA_HOME"
            };
            let saved = (key, std::env::var_os(key));
            std::env::set_var(key, root.path());
            Self {
                _lock: lock,
                _root: root,
                saved,
            }
        }

        pub(in crate::updater) fn directory(&self) -> PathBuf {
            crate::support_dirs::app_data_dir()
        }

        /// Download and staging leftovers in the app data folder: payload
        /// files, partial downloads and the staging manifest.
        pub(in crate::updater) fn staged_names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.directory())
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .filter(|name| {
                            name.contains("_download_")
                                || name.ends_with(".download")
                                || name.ends_with(".part")
                                || name == "staged_update.json"
                        })
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    impl Drop for AppData {
        fn drop(&mut self) {
            match self.saved.1.take() {
                Some(value) => std::env::set_var(self.saved.0, value),
                None => std::env::remove_var(self.saved.0),
            }
        }
    }

    /// A key pair for one test; its public half replaces the release key until
    /// the value drops.
    pub(in crate::updater) struct TestKey {
        pair: Ed25519KeyPair,
        _installed: crate::updater::TestReleaseKey,
    }

    impl TestKey {
        pub(in crate::updater) fn install() -> Self {
            let random = ring::rand::SystemRandom::new();
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).expect("a generated key pair");
            let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("the key pair parses");
            let public: [u8; 32] = pair
                .public_key()
                .as_ref()
                .try_into()
                .expect("a 32-byte public key");
            Self {
                pair,
                _installed: crate::updater::use_test_release_key(public),
            }
        }

        /// The text of `release-manifest.sig` for `manifest`: 128 hex
        /// characters and one LF.
        pub(in crate::updater) fn sign(&self, manifest: &[u8]) -> Vec<u8> {
            let mut text = hex(self.pair.sign(manifest).as_ref());
            text.push('\n');
            text.into_bytes()
        }
    }

    /// The signature text of `manifest` made with a key that is not the
    /// installed one (an attacker's own key).
    pub(in crate::updater) fn sign_with_another_key(manifest: &[u8]) -> Vec<u8> {
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).expect("a generated key pair");
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("the key pair parses");
        let mut text = hex(pair.sign(manifest).as_ref());
        text.push('\n');
        text.into_bytes()
    }

    /// One payload file of a release with its sidecar.
    #[derive(Clone)]
    pub(in crate::updater) struct Payload {
        pub(in crate::updater) role: &'static str,
        /// File name in a feed folder and asset name in the manifest.
        pub(in crate::updater) name: &'static str,
        /// Name in an HTTP path.
        pub(in crate::updater) http_name: &'static str,
        pub(in crate::updater) hash_name: &'static str,
        pub(in crate::updater) bytes: Vec<u8>,
    }

    impl Payload {
        pub(in crate::updater) fn sha256(&self) -> String {
            sha256_hex(&self.bytes)
        }
    }

    /// The four payloads of release `version` for this platform.
    pub(in crate::updater) fn payloads(version: &str) -> Vec<Payload> {
        [
            ("app", os::app_payload_spec()),
            ("updater", os::updater_payload_spec()),
            ("cli", os::cli_payload_spec()),
            ("launcher", os::launcher_payload_spec()),
        ]
        .into_iter()
        .map(|(role, spec)| Payload {
            role,
            name: spec.local_names[0],
            http_name: spec.http_names[0],
            hash_name: spec.hash_name,
            bytes: format!("{role} payload of release {version}; lokal-task fixture").into_bytes(),
        })
        .collect()
    }

    /// The canonical `release-manifest` for `version` over `assets` (name and
    /// bytes), names ordinal ascending.
    pub(in crate::updater) fn manifest(version: &str, assets: &[(&str, &[u8])]) -> Vec<u8> {
        let mut sorted: Vec<&(&str, &[u8])> = assets.iter().collect();
        sorted.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
        let mut text = format!("smart-explorer-release-manifest 1\nversion {version}\n");
        for (name, bytes) in sorted {
            text.push_str(&format!("{} {} {name}\n", sha256_hex(bytes), bytes.len()));
        }
        text.into_bytes()
    }

    /// A feed folder holding release `version`: the four payloads, their
    /// sidecars and `version.txt`.
    pub(in crate::updater) struct ReleaseFolder {
        dir: tempfile::TempDir,
        version: String,
        payloads: Vec<Payload>,
    }

    impl ReleaseFolder {
        pub(in crate::updater) fn new(version: &str) -> Self {
            let dir = tempfile::tempdir().expect("a feed folder");
            let payloads = payloads(version);
            for payload in &payloads {
                std::fs::write(dir.path().join(payload.name), &payload.bytes).expect("a payload");
                std::fs::write(
                    dir.path().join(payload.hash_name),
                    format!("{}  {}\n", payload.sha256(), payload.name),
                )
                .expect("a sidecar");
            }
            std::fs::write(dir.path().join("version.txt"), format!("{version}\n"))
                .expect("version.txt");
            Self {
                dir,
                version: version.to_string(),
                payloads,
            }
        }

        pub(in crate::updater) fn path(&self) -> &Path {
            self.dir.path()
        }

        pub(in crate::updater) fn feed(&self) -> Feed {
            Feed::Local(self.dir.path().to_path_buf())
        }

        pub(in crate::updater) fn payloads(&self) -> &[Payload] {
            &self.payloads
        }

        pub(in crate::updater) fn payload(&self, role: &str) -> &Payload {
            self.payloads
                .iter()
                .find(|payload| payload.role == role)
                .expect("a payload of this role")
        }

        pub(in crate::updater) fn write(&self, name: &str, bytes: &[u8]) {
            std::fs::write(self.dir.path().join(name), bytes).expect("a feed file");
        }

        pub(in crate::updater) fn read(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.dir.path().join(name)).expect("a feed file")
        }

        pub(in crate::updater) fn remove(&self, name: &str) {
            std::fs::remove_file(self.dir.path().join(name)).expect("a feed file");
        }

        /// Writes `release-manifest` for the folder's current payload bytes
        /// and `release-manifest.sig` over it, for `version`.
        pub(in crate::updater) fn sign_as(&self, key: &TestKey, version: &str) {
            let assets: Vec<(&str, &[u8])> = self
                .payloads
                .iter()
                .map(|payload| (payload.name, payload.bytes.as_slice()))
                .collect();
            let manifest = manifest(version, &assets);
            self.write("release-manifest.sig", &key.sign(&manifest));
            self.write("release-manifest", &manifest);
        }

        pub(in crate::updater) fn sign(&self, key: &TestKey) {
            self.sign_as(key, &self.version);
        }
    }
}

fn http(url: &str) -> Feed {
    Feed::Http(url.to_string())
}

/// E12-einstufung: which feed is the project's own, a plain http feed or
/// another feed, and what each needs.
#[test]
fn lokal_task_unsigned_feeds_are_classified_by_what_they_risk() {
    let official = http("https://github.com/b1ue-man/smart-explorer/releases/download/v1.2.3");
    let loopback = http("http://127.0.0.1:8080/feed");
    let own_tls = http("https://example.test/feed");
    let folder = Feed::Local(std::env::temp_dir());
    assert!(is_official_release_feed(&official));
    assert!(is_official_release_feed(&http(
        "HTTPS://GitHub.com/B1UE-MAN/Smart-Explorer/releases/latest/download"
    )));
    for not_official in [
        http("https://github.com/someone/smart-explorer/releases/download/v1.2.3"),
        http("https://github.com/b1ue-man/smart-explorer-fork/releases/download/v1.2.3"),
        http("https://github.com.example.test/b1ue-man/smart-explorer/releases/download/v1"),
        http("http://github.com/b1ue-man/smart-explorer/releases/download/v1.2.3"),
        http("https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/feed"),
        http("http://127.0.0.1:8080/feed"),
        http("https://example.test/feed"),
        Feed::Local(std::env::temp_dir()),
    ] {
        assert!(
            !is_official_release_feed(&not_official),
            "{}",
            not_official.display()
        );
    }

    let cases = [
        (
            "the project's release",
            unsigned_kind(&official),
            UnsignedFeed::OfficialRelease,
            true,
            "unsigned_release",
        ),
        (
            "a plain http feed",
            unsigned_kind(&loopback),
            UnsignedFeed::PlainHttp,
            true,
            "unsigned_http_feed",
        ),
        (
            "an own https feed",
            unsigned_kind(&own_tls),
            UnsignedFeed::Custom,
            false,
            "unsigned_feed",
        ),
        (
            "a folder",
            unsigned_kind(&folder),
            UnsignedFeed::Custom,
            false,
            "unsigned_feed",
        ),
    ];
    for (case, kind, expected, needs_consent, code) in cases {
        assert_eq!(kind, expected, "{case}");
        assert_eq!(kind.needs_consent(), needs_consent, "{case}");
        assert_eq!(kind.code(), code, "{case}");
        assert!(!kind.explanation().is_empty(), "{case}");
    }
    assert!(UnsignedFeed::PlainHttp
        .explanation()
        .contains("unsigned http feed"));
    // The code is the stable text of the serialized value.
    assert_eq!(
        serde_json::to_string(&UnsignedFeed::PlainHttp).expect("serialized"),
        "\"plain_http\""
    );
}

/// E12-einstufung: consent covers exactly one version, never a signed
/// release's manifest, and never an unsigned own feed.
#[test]
fn lokal_task_consent_covers_exactly_the_named_version() {
    let none = UnsignedConsent::none();
    let plain = FeedTrust::Unsigned(UnsignedFeed::PlainHttp);
    let official = FeedTrust::Unsigned(UnsignedFeed::OfficialRelease);
    let custom = FeedTrust::Unsigned(UnsignedFeed::Custom);
    assert_eq!(
        plain.missing_consent("1.2.3", &none),
        Some(UnsignedFeed::PlainHttp)
    );
    assert_eq!(
        official.missing_consent("1.2.3", &none),
        Some(UnsignedFeed::OfficialRelease)
    );
    assert_eq!(custom.missing_consent("1.2.3", &none), None);
    let for_one = UnsignedConsent::for_version("1.2.3");
    assert_eq!(plain.missing_consent("1.2.3", &for_one), None);
    assert_eq!(official.missing_consent("1.2.3", &for_one), None);
    assert_eq!(
        plain.missing_consent("1.2.4", &for_one),
        Some(UnsignedFeed::PlainHttp)
    );
    assert_eq!(
        official.missing_consent("1.2.3-rc1", &for_one),
        Some(UnsignedFeed::OfficialRelease)
    );
    assert_eq!(plain.unsigned(), Some(UnsignedFeed::PlainHttp));
    assert!(plain.manifest().is_none());

    let error = consent_error(
        UnsignedFeed::PlainHttp,
        "1.2.3",
        &http("http://h.test/feed"),
    );
    assert!(error.starts_with("unsigned_http_feed:"), "{error}");
    assert!(error.contains("unsigned http feed"), "{error}");
    assert!(error.contains("--allow-unsigned"), "{error}");
    assert!(error.contains("v1.2.3"), "{error}");
}

/// `read_trust` reads the two signature files of the feed and nothing else:
/// over loopback HTTP and from a folder, without them the feed is unsigned
/// by its own kind; a 404 is "absent", any other failure is an error.
#[test]
fn lokal_task_feed_without_manifest_is_unsigned_by_its_kind() {
    let _app = AppData::new();
    let server = HttpFeed::start();
    let trust = read_trust(&server.feed(), VERSION).expect("an unsigned plain http feed");
    assert_eq!(trust.unsigned(), Some(UnsignedFeed::PlainHttp));
    assert_eq!(
        server.requests(),
        ["release-manifest", "release-manifest.sig"],
        "only the two signature files are read"
    );

    let folder = ReleaseFolder::new(VERSION);
    let trust = read_trust(&folder.feed(), VERSION).expect("an unsigned folder feed");
    assert_eq!(trust.unsigned(), Some(UnsignedFeed::Custom));
    assert!(trust
        .missing_consent(VERSION, &UnsignedConsent::none())
        .is_none());

    // A server error is no "unsigned": it is an error, never a fallback.
    server.serve(
        "release-manifest",
        Route {
            body: Vec::new(),
            announce: Some(10),
            cut_at: Some(0),
        },
    );
    assert!(read_trust(&server.feed(), VERSION).is_err());
}
