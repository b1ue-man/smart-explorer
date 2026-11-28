//! `root`: the session's working root, set once like the folder of an open
//! explorer tab (plan 2026-11-27 section C; user 2026-11-27: "n remote setzen
//! kann als ziel einmal und dann read edit write operationen sowie ls und
//! andere file system operationen dort direkt ausführen"). `set` checks that
//! the target is a reachable folder and reports what it is, which device or
//! server holds it and how it is reached; afterwards every file tool takes
//! paths relative to it and `exec` without `target` runs on its device or
//! server. `show` reports it, `clear` removes it.
use super::args::Args;
use super::response;
use super::root_path::{RootKind, WorkRoot};
use super::session::Session;
use crate::creds::{Protocol, SavedConnection};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

pub(super) const DESCRIPTION: &str = "Sets the session's working root, kept open like an explorer tab: `set` with a folder `target` of any kind (local, `@label:/path`, saved endpoint, `share://direct/<id>/path`, `share://room/<room>/<device>/path`, `gdrive:///path`, `vault://<site>/path`; a relative one is taken below the current root) checks that it is a reachable folder and reports its `kind`, the device or server holding it and how it is reached (`via`, for Share peers the current `route`). Afterwards every file tool (list, stat, read, write, edit, mkdir, remove, copy, move, search, grep) takes paths relative to it (`src/main.rs`; `.` or an empty target = the root; `..` never leaves it), absolute targets still work, and `exec` without `target` runs on its SSH remote or Share peer. The connection stays open for the session and is opened again after it failed. `show` reports the root, `clear` removes it. Removing the root itself needs `allow_root`.";

pub(super) fn properties() -> Value {
    json!({
        "action": {"type": "string", "enum": ["set", "show", "clear"], "description": "set, show (default) or clear."},
        "target": {"type": "string", "description": "For set: the folder to work in, absolute or relative to the current root."}
    })
}

pub(super) fn root(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    match args.opt_string("action")?.as_deref().unwrap_or("show") {
        "set" => {
            let spec = args.string("target")?;
            // Se's own data is no working root while mask lists are active.
            let resolved = session.targets.resolve(&spec)?;
            super::mask_layer_guard::check(
                session,
                &resolved,
                super::mask_layer_guard::Reach::Entry,
            )?;
            drop(resolved);
            let (root, resolved) = session.targets.set_root(&spec)?;
            let mut value = describe(&root);
            if let Value::Object(fields) = &mut value {
                fields.insert("set".into(), json!(true));
                fields.insert("name".into(), json!(resolved_name(&resolved.path)));
            }
            Ok(response::ok(value))
        }
        "show" => Ok(response::ok(match session.targets.root() {
            Some(root) => describe(&root),
            None => {
                json!({ "root": null, "note": "no working root; `root` with action set and a folder target sets one" })
            }
        })),
        "clear" => {
            let cleared = session.targets.clear_root();
            Ok(response::ok(json!({
                "root": null,
                "cleared": cleared.map(|root| root.spec.clone()),
            })))
        }
        other => Err(format!("unknown action `{other}`; use set, show or clear")),
    }
}

fn resolved_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("/")
}

/// Where `exec` without `target` runs for the working root.
pub(super) struct ExecHome {
    /// The `exec` target: the root itself for an SSH remote (its path is the
    /// working directory), the peer for a Share root.
    pub(super) selector: String,
    /// For a Share root below the peer's top level: its Share path, which
    /// the peer does not translate into a working directory.
    pub(super) share_path: Option<String>,
}

pub(super) fn exec_home(root: &WorkRoot) -> Result<ExecHome, String> {
    match root.kind {
        RootKind::Login(Protocol::Sftp) => Ok(ExecHome {
            selector: root.spec.clone(),
            share_path: None,
        }),
        RootKind::Peer => {
            let normalized = root.spec.replace('\\', "/");
            let (peer, path) = crate::share::PeerOpenTarget::from_endpoint(&normalized)
                .ok_or_else(|| format!("{} is no Share endpoint", root.spec))?;
            Ok(ExecHome {
                selector: peer.endpoint_prefix(),
                share_path: (path.trim_matches('/') != "").then_some(path),
            })
        }
        kind => Err(format!(
            "the working root {} is a {} location; `exec` runs on saved SFTP remotes (SSH) and Share peers: give `target`",
            root.spec,
            kind.label(&root.spec)
        )),
    }
}

/// What `set` and `show` report about the root.
pub(super) fn describe(root: &WorkRoot) -> Value {
    let exec = match root.kind {
        RootKind::Login(Protocol::Sftp) => json!("ssh"),
        RootKind::Peer => json!("share"),
        _ => Value::Null,
    };
    let mut value = json!({
        "root": root.spec,
        "kind": root.kind.label(&root.spec),
        "path": root.path,
        "exec": exec,
        "connection": "kept open for this session; opened again after a failure",
    });
    let details = match root.kind {
        RootKind::Local => json!({ "device": "this device", "via": "local file system" }),
        RootKind::Unc => json!({
            "server": unc_server(&root.spec),
            "via": "Windows share (SMB) through this device's network login",
        }),
        RootKind::Drive => {
            json!({ "server": "Google Drive", "via": "Google Drive API over HTTPS" })
        }
        RootKind::Peer => peer_details(&root.spec),
        RootKind::Vault => vault_details(&root.spec),
        RootKind::Login(_) | RootKind::Smb | RootKind::WindowsShare | RootKind::Url => {
            saved_details(&root.spec)
        }
    };
    if let (Value::Object(fields), Value::Object(more)) = (&mut value, details) {
        fields.extend(more);
    }
    if root.kind == RootKind::Peer && root.path.trim_matches('/') != "" {
        if let Value::Object(fields) = &mut value {
            fields.insert(
                "exec_note".into(),
                json!("exec without `cwd` cannot start in this Share folder: the peer does not tell its disk path; pass `cwd` with the peer's own path"),
            );
        }
    }
    value
}

fn unc_server(spec: &str) -> String {
    let normalized = spec.replace('\\', "/");
    normalized
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn saved_details(spec: &str) -> Value {
    let saved = if spec.starts_with('@') {
        crate::cli::target::saved_shorthand(spec).ok().flatten()
    } else {
        crate::connect::saved_and_path(&spec.replace('\\', "/"))
    };
    match saved {
        Some((connection, _)) => saved_value(&connection),
        None => json!({ "server": null, "via": "a remote URL without a saved connection" }),
    }
}

fn saved_value(connection: &SavedConnection) -> Value {
    let address = format!(
        "{}@{}:{}",
        connection.user, connection.host, connection.port
    );
    let via = match connection.protocol {
        Protocol::Sftp if connection.use_agent => {
            format!("SFTP over SSH to {address}, with the se-agent on the server")
        }
        Protocol::Sftp => format!("SFTP over SSH to {address}"),
        Protocol::Ftp => format!("FTP to {address}"),
        Protocol::Ftps => format!("FTPS (TLS) to {address}"),
        Protocol::Webdav => format!("WebDAV over HTTPS to {address}"),
        Protocol::Smb => format!("SMB to {address}"),
        Protocol::Share => format!("Windows share {} (net use)", connection.root),
    };
    json!({
        "server": connection.host,
        "label": connection.display(),
        "account": connection.account(),
        "protocol": connection.protocol.as_str(),
        "via": via,
    })
}

fn vault_details(spec: &str) -> Value {
    let site = crate::vault::VaultLocator::parse(spec)
        .map(|locator| locator.site)
        .ok();
    let record = site.as_deref().and_then(|site| {
        crate::vault::RegistryStore::open_default()
            .resolve(site)
            .ok()
    });
    match record {
        Some(record) => json!({
            "vault": record.label,
            "location": record.location,
            "via": format!("the unlocked vault over its storage {}", record.location),
        }),
        None => json!({ "vault": site, "via": "a vault of this device's vault list" }),
    }
}

/// The peer as `connections` lists it, and the routes the background worker
/// reports for it now (direct or relay).
fn peer_details(spec: &str) -> Value {
    let normalized = spec.replace('\\', "/");
    let Some((peer, _)) = crate::share::PeerOpenTarget::from_endpoint(&normalized) else {
        return json!({});
    };
    let endpoint = peer.endpoint_prefix();
    let row = crate::cli::connections::connections_value()
        .ok()
        .and_then(|rows| match rows {
            Value::Array(rows) => rows.into_iter().find(|row| {
                row["endpoint"].as_str().or_else(|| row["account"].as_str())
                    == Some(endpoint.as_str())
            }),
            _ => None,
        });
    let (route, route_error) = match crate::daemon::share_worker_snapshot() {
        Ok(snapshot) => (peer_route(&peer, snapshot.routes.as_ref()), None),
        Err(error) => (Value::Null, Some(error)),
    };
    json!({
        "device": row.as_ref().map(|row| row["label"].clone()),
        "device_status": row.as_ref().map(|row| row["status"].clone()),
        "via": "Smart Explorer Share through the background worker (direct between the devices first, relay as fallback)",
        "route": route,
        "route_error": route_error,
    })
}

fn peer_route(
    peer: &crate::share::PeerOpenTarget,
    routes: Option<&crate::share::ShareRouteSnapshot>,
) -> Value {
    let Some(routes) = routes else {
        return Value::Null;
    };
    let found = routes.peers.iter().find(|device| match peer {
        crate::share::PeerOpenTarget::Direct { contact_id } => {
            device.contact_id.as_deref() == Some(contact_id.as_str())
        }
        crate::share::PeerOpenTarget::RoomDevice { device_id, .. } => {
            &device.device_id == device_id
        }
    });
    match found {
        Some(device) => Value::Array(
            device
                .connections
                .iter()
                .map(|connection| {
                    json!({
                        "label": connection.label,
                        "state": connection.state,
                        "path": connection.path_class,
                    })
                })
                .collect(),
        ),
        None => json!([]),
    }
}
