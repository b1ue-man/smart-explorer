//! Inputs come only from the suite-owned server manifest, never implicit hosts.
use crate::creds::{AuthKind, Protocol, SavedConnection};
use crate::vfs::{Backend, BackendHandle};
use serde::Deserialize;
use std::io::{Read, Write};

#[derive(Deserialize)]
struct Manifest {
    providers: Vec<Record>,
    zip: String,
}

#[derive(Deserialize)]
struct Record {
    name: String,
    protocol: String,
    endpoint: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    password: Option<String>,
    key_path: Option<String>,
    root: Option<String>,
    #[serde(default)]
    agent: bool,
}

pub(crate) struct Provider {
    pub(crate) name: String,
    pub(crate) endpoint: String,
    pub(crate) agent: bool,
}

fn manifest() -> Manifest {
    let path = std::env::var_os("SE_SYNC_PROVIDER_MANIFEST")
        .expect("C04 requires suite-owned real protocol fixture manifest");
    serde_json::from_slice(&std::fs::read(path).expect("read provider manifest"))
        .expect("valid provider fixture manifest")
}

pub(crate) fn providers() -> Vec<Provider> {
    manifest()
        .providers
        .into_iter()
        .map(|record| {
            let endpoint = if let Some(endpoint) = record.endpoint {
                endpoint
            } else {
                let saved = SavedConnection {
                    protocol: Protocol::parse(&record.protocol)
                        .expect("supported fixture protocol"),
                    host: record.host.expect("fixture host"),
                    port: record.port.expect("fixture discovered port"),
                    user: record.user.expect("fixture user"),
                    auth: match record.key_path {
                        Some(path) => {
                            assert_eq!(record.protocol, "sftp", "key fixture must use SFTP");
                            assert!(
                                std::path::Path::new(&path).is_file(),
                                "owned private key missing"
                            );
                            AuthKind::Key { path }
                        }
                        None => AuthKind::Password,
                    },
                    root: record.root.expect("fixture writable root"),
                    label: format!("sync-task-{}", record.name),
                    use_agent: record.agent,
                };
                crate::creds::save_connection_with_secret(&saved, record.password.as_deref())
                    .expect("persist normal fixture connection and credential");
                let reloaded =
                    crate::creds::load_connections_checked().expect("reload fixture connections");
                assert!(reloaded
                    .iter()
                    .any(|entry| entry.account() == saved.account()
                        && entry.root == saved.root
                        && entry.use_agent == saved.use_agent
                        && entry.auth == saved.auth));
                if let Some(secret) = record
                    .password
                    .as_deref()
                    .filter(|secret| !secret.is_empty())
                {
                    assert!(
                        crate::creds::get_secret_checked(&saved.account())
                            .expect("reload normal fixture credential")
                            .as_deref()
                            == Some(secret),
                        "stored fixture password/key passphrase was not preserved"
                    );
                }
                saved.to_target()
            };
            Provider {
                name: record.name,
                endpoint,
                agent: record.agent,
            }
        })
        .collect()
}

impl Provider {
    /// Reopen through production persisted locator resolution; create only this child.
    pub(crate) fn open(&self, child: &str) -> (BackendHandle, String) {
        assert!(!child.contains(['/', '\\']) && !child.is_empty());
        let (backend, root) = super::resolution::resolve_endpoint(&self.endpoint)
            .unwrap_or_else(|error| panic!("{} saved resolver: {error}", self.name));
        if self.agent {
            assert!(
                backend.supports_walk_hashed() && backend.supports_walk_tree(),
                "{} silently fell back to plain SFTP",
                self.name
            );
        }
        let subroot =
            crate::vfs::sync_child_path(&*backend, &root, child).expect("literal child root");
        backend.mkdir_all(&subroot).expect("create owned sync root");
        if self.name == "drive" {
            // The exact cfg(test) Drive locator is registered by the caller.
            // All HTTP and engine operations still use the real Drive backend.
            return (backend, subroot);
        }
        let locator = format!("{}/{}", self.endpoint.trim_end_matches(['/', '\\']), child);
        let (reopened, reopened_root) = super::resolution::resolve_endpoint(&locator)
            .unwrap_or_else(|error| panic!("{} saved child locator: {error}", self.name));
        assert_eq!(reopened_root.replace('\\', "/"), subroot.replace('\\', "/"));
        assert_eq!(
            backend.namespace_identity(),
            reopened.namespace_identity(),
            "saved child locator changed provider identity"
        );
        if self.agent {
            assert!(
                reopened.supports_walk_hashed() && reopened.supports_walk_tree(),
                "saved child locator fell back to plain SFTP"
            );
        }
        (reopened, reopened_root)
    }
}

pub(crate) fn write(backend: &dyn Backend, root: &str, relative: &str, bytes: &[u8]) {
    if let Some((parent, _)) = relative.rsplit_once('/') {
        backend
            .mkdir_all(&path(backend, root, parent))
            .expect("fixture nested directory");
    }
    let path = path(backend, root, relative);
    let mut writer = backend.open_write(&path).expect("fixture write");
    writer.write_all(bytes).expect("fixture complete bytes");
    writer.flush().expect("fixture publish write");
    drop(writer);
    backend.invalidate_cache();
}

pub(crate) fn read(backend: &dyn Backend, root: &str, relative: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    backend
        .open_read(&path(backend, root, relative))
        .expect("actual provider read")
        .read_to_end(&mut bytes)
        .expect("complete provider read");
    bytes
}

pub(crate) fn path(backend: &dyn Backend, root: &str, relative: &str) -> String {
    let mut path = root.to_string();
    for name in relative.split('/') {
        path = crate::vfs::sync_child_path(backend, &path, name).expect("literal provider path");
    }
    path
}

pub(crate) fn zip() -> crate::zipfs::ZipBackend {
    crate::zipfs::ZipBackend::open(&manifest().zip).expect("owned readonly ZIP source")
}
