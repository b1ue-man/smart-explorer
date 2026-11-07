//! Offline provisioning of the gateway network (E14, L-B2; suite expectations
//! E14-f and (c) to (e) of `f-lokal-2`). `use_default_relays: true` is
//! resolved into the relays of the Share server stored in this program, never
//! into number0; `provision_network` writes only the network file and refuses
//! the active installation and state. The write itself needs root (Linux) or an
//! administrator (Windows) and a private ancestry; the tests accept the
//! platform's refusal and then require that nothing was left behind.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use iroh::RelayUrl;

use super::{provision_network, resolve_network};
use crate::login::core::gateway_types::GatewayNetwork;
use crate::login::core::model::VERSION;
use crate::login::os::platform;

const PIN_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const LOCATION_REFUSAL: &str =
    "offline provisioning must not overwrite or contain active service installation/state";

static ENVIRONMENT: Mutex<()> = Mutex::new(());

/// The app data of this program in a new directory, without relay overrides,
/// for one test; the environment is put back afterwards.
struct IsolatedAppData {
    _lock: MutexGuard<'static, ()>,
    _root: tempfile::TempDir,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl IsolatedAppData {
    fn new() -> Self {
        let lock = ENVIRONMENT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = tempfile::tempdir().expect("a temporary app data root");
        let data_home = if cfg!(windows) {
            "APPDATA"
        } else {
            "XDG_DATA_HOME"
        };
        let keys = [data_home, "SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"];
        let saved = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in &keys[1..] {
            std::env::remove_var(key);
        }
        std::env::set_var(data_home, root.path());
        Self {
            _lock: lock,
            _root: root,
            saved,
        }
    }

    fn store_server(&self, text: &str) {
        std::fs::write(crate::support_dirs::app_data_file("share_server.txt"), text)
            .expect("the stored Share server address");
    }
}

impl Drop for IsolatedAppData {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn pin() -> [u8; 32] {
    std::array::from_fn(|index| ((index % 16) * 17) as u8)
}

fn relay(text: &str) -> RelayUrl {
    text.parse().expect("a relay URL")
}

fn default_network() -> GatewayNetwork {
    GatewayNetwork {
        format: VERSION,
        use_default_relays: true,
        relay_urls: Vec::new(),
        relay_pins: Vec::new(),
        allow_plaintext_relays: false,
    }
}

/// E14-f, (c) and (d): "default relays" become the relays, pins and plaintext
/// rule of the stored Share server; without a server there are no relays; an
/// explicit network is not touched.
#[test]
fn lokal_task_provisioning_resolves_default_relays_into_the_configured_server() {
    let app = IsolatedAppData::new();

    // (d): no stored server, only direct paths.
    let none = resolve_network(default_network()).expect("a network without a server");
    assert!(!none.use_default_relays);
    assert!(none.relay_urls.is_empty());
    assert!(none.relay_pins.is_empty());
    assert!(!none.allow_plaintext_relays);
    assert_eq!(none.format, VERSION);

    // (c) and E14-f: a TLS server with a pin.
    app.store_server(&format!(
        "wss://share.example.test/se-share#sha256={PIN_HEX}"
    ));
    let resolved = resolve_network(default_network()).expect("the configured server's network");
    assert!(!resolved.use_default_relays);
    assert_eq!(resolved.relay_urls.len(), 1);
    assert_eq!(
        relay(&resolved.relay_urls[0]),
        relay("https://share.example.test")
    );
    assert_eq!(resolved.relay_pins, vec![pin()]);
    assert!(!resolved.allow_plaintext_relays);
    resolved.validate().expect("the resolved file is valid");
    let written: serde_json::Value = serde_json::to_value(&resolved).expect("the file form");
    assert_eq!(written["use_default_relays"], false);
    assert!(written["relay_urls"].is_array());
    assert!(written["relay_pins"].is_array());
    assert_eq!(written["allow_plaintext_relays"], false);

    // A plaintext server keeps its plaintext rule.
    app.store_server("tcp://192.0.2.7:51820");
    let plain = resolve_network(default_network()).expect("the plaintext server's network");
    assert!(plain.allow_plaintext_relays);
    assert_eq!(plain.relay_urls.len(), 1);
    assert_eq!(relay(&plain.relay_urls[0]), relay("http://192.0.2.7:51821"));

    // An explicit network is returned as it is, whatever server is stored.
    let explicit = GatewayNetwork {
        format: VERSION,
        use_default_relays: false,
        relay_urls: vec!["https://explicit.example.test".into()],
        relay_pins: vec![[7; 32]],
        allow_plaintext_relays: false,
    };
    assert_eq!(
        resolve_network(explicit.clone()).expect("an explicit network"),
        explicit
    );
}

/// (c) and (d) through `provision_network`: with the right the one file
/// `gateway-network.json` appears with the resolved network; without it the
/// platform refuses and no directory is left. Either way the refusal is not
/// the location refusal.
#[test]
fn lokal_task_provision_network_writes_only_the_network_file_or_leaves_nothing() {
    let app = IsolatedAppData::new();
    let output = tempfile::tempdir().expect("an output root");
    for (case, server, expected) in [
        (
            "configured server",
            Some(format!("wss://share.example.test#sha256={PIN_HEX}")),
            GatewayNetwork {
                format: VERSION,
                use_default_relays: false,
                relay_urls: vec!["https://share.example.test".into()],
                relay_pins: vec![pin()],
                allow_plaintext_relays: false,
            },
        ),
        (
            "no server",
            None,
            GatewayNetwork {
                format: VERSION,
                use_default_relays: false,
                relay_urls: Vec::new(),
                relay_pins: Vec::new(),
                allow_plaintext_relays: false,
            },
        ),
    ] {
        match &server {
            Some(text) => app.store_server(text),
            None => {
                let _ =
                    std::fs::remove_file(crate::support_dirs::app_data_file("share_server.txt"));
            }
        }
        let directory = output.path().join(case.replace(' ', "-"));
        match provision_network(directory.clone()) {
            Ok(done) => {
                assert_eq!(done.network_file, directory.join("gateway-network.json"));
                assert_eq!(
                    done.network.relay_urls.len(),
                    expected.relay_urls.len(),
                    "{case}"
                );
                assert_eq!(done.network.relay_pins, expected.relay_pins, "{case}");
                assert!(!done.network.use_default_relays, "{case}");
                let stored: GatewayNetwork = serde_json::from_slice(
                    &std::fs::read(&done.network_file).expect("the network file"),
                )
                .expect("the network file parses");
                assert_eq!(stored, done.network, "{case}");
                let mut names: Vec<String> = std::fs::read_dir(&directory)
                    .expect("the bundle")
                    .map(|entry| {
                        entry
                            .expect("an entry")
                            .file_name()
                            .to_string_lossy()
                            .into()
                    })
                    .collect();
                names.sort();
                assert_eq!(names, ["gateway-network.json"], "{case}");
            }
            Err(error) => {
                assert!(
                    !error.to_string().contains(LOCATION_REFUSAL),
                    "{case}: refused as an active location: {error}"
                );
                assert!(
                    !directory.exists(),
                    "{case}: a refused provisioning left a directory ({error})"
                );
            }
        }
    }
}

/// (e): the active installation, its release root and the gateway state are
/// never overwritten or contained, for `provision_network` as for `provision`.
#[test]
fn lokal_task_provision_network_refuses_the_active_installation_and_state() {
    let _app = IsolatedAppData::new();
    let active = platform::paths().expect("the installation paths");
    let state = platform::gateway_state_directory().expect("the gateway state directory");
    let parent_of_config: PathBuf = active
        .config
        .parent()
        .expect("the configuration root has a parent")
        .to_path_buf();
    let candidates: Vec<(&str, PathBuf)> = vec![
        ("the configuration root", active.config.clone()),
        ("below the configuration root", active.config.join("bundle")),
        ("the release root", active.releases.clone()),
        ("below the release root", active.releases.join("v1")),
        ("the gateway state", state.clone()),
        ("below the gateway state", state.join("bundle")),
        (
            "a directory that contains the configuration root",
            parent_of_config,
        ),
    ];
    for (case, directory) in candidates {
        let existed = Path::exists(&directory);
        let error = provision_network(directory.clone())
            .err()
            .unwrap_or_else(|| panic!("{case}: the location was accepted"));
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{case}");
        assert_eq!(error.to_string(), LOCATION_REFUSAL, "{case}");
        assert_eq!(Path::exists(&directory), existed, "{case}");
    }
}
