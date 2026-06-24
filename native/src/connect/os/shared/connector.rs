use crate::connect::endpoint::{enc, ep_prefix, is_remote_url, norm_root, parse_remote_url};
use crate::connect::persistence::{build_saved, persist};
use crate::connect::{ConnectForm, ConnectResult, Connected, RemoteState};
use crate::creds::Protocol;
use crate::vfs::BackendHandle;
use crossbeam_channel::{unbounded, Receiver};
use std::sync::Arc;

/// Connect off the UI thread; the app drains the single result.
pub fn spawn_connect(form: ConnectForm, secret: Option<String>) -> Receiver<ConnectResult> {
    let (tx, rx) = unbounded();
    std::thread::Builder::new()
        .name("connect".into())
        .spawn(move || {
            let _ = tx.send(do_connect(form, secret));
        })
        .ok();
    rx
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn label_for(form: &ConnectForm, port: u16) -> String {
    if !form.label.trim().is_empty() {
        return form.label.trim().to_string();
    }
    match form.protocol {
        Protocol::Share => form.unc.trim().to_string(),
        _ => format!(
            "{}://{}@{}:{}",
            form.protocol.as_str(),
            form.user,
            form.host,
            port
        ),
    }
}

fn do_connect(form: ConnectForm, secret: Option<String>) -> ConnectResult {
    let port: u16 = form
        .port
        .trim()
        .parse()
        .unwrap_or_else(|_| form.protocol.default_port());

    match form.protocol {
        Protocol::Sftp => connect_sftp(form, secret, port),
        Protocol::Ftp | Protocol::Ftps => connect_ftp(form, secret, port),
        Protocol::Webdav => connect_webdav(form, secret, port),
        Protocol::Share => connect_share(form, secret, port),
    }
}

fn connect_sftp(form: ConnectForm, secret: Option<String>, port: u16) -> ConnectResult {
    // A saved-connection secret (keyring) overrides an empty form field.
    let password = secret.clone().unwrap_or_else(|| form.password.clone());
    let passphrase = secret.clone().unwrap_or_else(|| form.passphrase.clone());
    let auth = if form.use_key {
        crate::sftp::SftpAuth::Key {
            path: form.keyfile.trim().to_string(),
            passphrase: opt(&passphrase),
        }
    } else {
        crate::sftp::SftpAuth::Password(password.clone())
    };
    let root = norm_root(&form.root);
    let cfg = crate::sftp::SftpConfig {
        host: form.host.trim().to_string(),
        port,
        user: form.user.trim().to_string(),
        auth,
        root: root.clone(),
    };
    match crate::sftp::SftpBackend::connect(cfg) {
        Ok(be) => {
            let s = if form.use_key { passphrase } else { password };
            persist(&form, port, Some(&s));
            let label = label_for(&form, port);
            // Opt-in: try to deploy + use the SSH remote agent (#24). Any
            // failure (no bundled binary, no exec right, ...) falls back to
            // plain SFTP, so connecting never breaks.
            let be_arc: Arc<crate::sftp::SftpBackend> = Arc::new(be);
            let sftp_handle = be_arc.clone(); // kept for later agent activation
            let account = Some(build_saved(&form, port).account());
            let (backend, agent_version): (BackendHandle, Option<String>) = if form.use_agent {
                let inner: BackendHandle = be_arc.clone();
                match crate::agent::deploy_over_sftp(&be_arc, inner) {
                    Ok(agent) => {
                        let ver = agent.version().to_string();
                        (Arc::new(agent), Some(ver))
                    }
                    Err(_) => (be_arc, None), // fall back to plain SFTP
                }
            } else {
                (be_arc, None)
            };
            ConnectResult::Ok(Connected {
                remote: Some(RemoteState {
                    backend,
                    label: label.clone(),
                    agent_version,
                    zip_return: None,
                    sftp: Some(sftp_handle),
                    account,
                    endpoint_prefix: ep_prefix(&form, port),
                }),
                net: None,
                target: root,
                label,
            })
        }
        Err(e) => ConnectResult::Err(e.to_string()),
    }
}

fn connect_ftp(form: ConnectForm, secret: Option<String>, port: u16) -> ConnectResult {
    let scheme = if form.protocol == Protocol::Ftps {
        "ftps"
    } else {
        "ftp"
    };
    let password = secret.clone().unwrap_or_else(|| form.password.clone());
    let user = form.user.trim();
    let userinfo = if password.is_empty() {
        enc(user)
    } else {
        format!("{}:{}", enc(user), enc(&password))
    };
    let root = norm_root(&form.root);
    let url = format!(
        "{scheme}://{}@{}:{}{}",
        userinfo,
        form.host.trim(),
        port,
        root
    );
    match crate::ftp::backend_from_url(&url) {
        Ok(be) => {
            persist(&form, port, Some(&password));
            let label = label_for(&form, port);
            ConnectResult::Ok(Connected {
                remote: Some(RemoteState {
                    backend: Arc::new(be),
                    label: label.clone(),
                    agent_version: None,
                    zip_return: None,
                    sftp: None,
                    account: None,
                    endpoint_prefix: ep_prefix(&form, port),
                }),
                net: None,
                target: root,
                label,
            })
        }
        Err(e) => ConnectResult::Err(e.to_string()),
    }
}

fn connect_webdav(form: ConnectForm, secret: Option<String>, port: u16) -> ConnectResult {
    let password = secret.clone().unwrap_or_else(|| form.password.clone());
    let root = norm_root(&form.root);
    let cfg = crate::webdav::WebdavConfig {
        https: true,
        host: form.host.trim().to_string(),
        port,
        user: form.user.trim().to_string(),
        password: password.clone(),
        root: root.clone(),
    };
    match crate::webdav::WebdavBackend::connect(cfg) {
        Ok(be) => {
            persist(&form, port, Some(&password));
            let label = label_for(&form, port);
            ConnectResult::Ok(Connected {
                remote: Some(RemoteState {
                    backend: Arc::new(be),
                    label: label.clone(),
                    agent_version: None,
                    zip_return: None,
                    sftp: None,
                    account: None,
                    endpoint_prefix: ep_prefix(&form, port),
                }),
                net: None,
                target: root,
                label,
            })
        }
        Err(e) => ConnectResult::Err(e.to_string()),
    }
}

fn connect_share(form: ConnectForm, secret: Option<String>, port: u16) -> ConnectResult {
    let unc = form.unc.trim().to_string();
    let password = secret.clone().unwrap_or_else(|| form.password.clone());
    match crate::net::NetConnection::connect(
        &unc,
        opt(&form.user).as_deref(),
        opt(&password).as_deref(),
    ) {
        Ok(nc) => {
            persist(&form, port, Some(&password));
            let label = label_for(&form, port);
            ConnectResult::Ok(Connected {
                remote: None,
                net: Some(nc),
                target: unc,
                label,
            })
        }
        Err(e) => ConnectResult::Err(e.to_string()),
    }
}

/// Open Google Drive at `path` as a backend (uses the stored OAuth token).
/// Blocks on the network - call off the UI thread.
pub fn open_gdrive(path: &str) -> Result<(BackendHandle, String), String> {
    let be = crate::gdrive::GDriveBackend::connect(path)?;
    let root = if path.trim().is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };
    Ok((Arc::new(be), root))
}

/// Open a saved connection at `path` (synchronous; blocks on the network - call
/// off the UI thread). Reuses the connection's stored credentials (keyring).
/// Returns the live backend + the navigated root path.
pub fn open_saved_at(
    c: &crate::creds::SavedConnection,
    path: &str,
) -> Result<(BackendHandle, String), String> {
    if !c.protocol.is_url() {
        // Share: the UNC is browsed locally once authenticated.
        let secret = crate::creds::get_secret(&c.account());
        let mut form = ConnectForm::from_saved(c);
        form.save = false;
        match do_connect(form, secret) {
            ConnectResult::Ok(conn) => Ok((
                Arc::new(crate::vfs::LocalBackend::new(&conn.target)),
                conn.target,
            )),
            ConnectResult::Err(e) => Err(e),
        }
    } else {
        let secret = crate::creds::get_secret(&c.account());
        let mut form = ConnectForm::from_saved(c);
        form.root = if path.is_empty() {
            "/".into()
        } else {
            path.to_string()
        };
        form.save = false;
        match do_connect(form, secret) {
            ConnectResult::Ok(conn) => match conn.remote {
                Some(rs) => Ok((rs.backend, conn.target)),
                None => Err("Endpoint ist keine Remote-Verbindung".into()),
            },
            ConnectResult::Err(e) => Err(e),
        }
    }
}

/// Resolve a sync endpoint into a live backend + root. Local/UNC paths ->
/// `LocalBackend`; remote URLs -> re-open the matching saved connection. Blocks
/// on the network for remote endpoints, so run it off the UI thread.
pub fn resolve_endpoint(endpoint: &str) -> Result<(BackendHandle, String), String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err("Leerer Pfad".into());
    }
    if let Some((target, root)) = crate::share::PeerOpenTarget::from_endpoint(endpoint) {
        let (_label, backend, _status) = crate::daemon::open_share_backend(target)?;
        return Ok((backend, root));
    }
    if !is_remote_url(endpoint) {
        return Ok((
            Arc::new(crate::vfs::LocalBackend::new(endpoint)),
            endpoint.to_string(),
        ));
    }
    // Google Drive: gdrive:///<path> -> re-open from the stored OAuth token.
    if let Some(rest) = endpoint.strip_prefix("gdrive://") {
        let path = format!("/{}", rest.trim_start_matches('/'));
        return open_gdrive(&path);
    }
    let (proto, user, host, port, path) =
        parse_remote_url(endpoint).ok_or_else(|| "Ungültige Remote-Adresse".to_string())?;
    let conns = crate::creds::load_connections();
    let c = conns
        .iter()
        .find(|c| c.protocol == proto && c.user == user && c.host == host && c.port == port)
        .ok_or_else(|| {
            "Keine gespeicherte Verbindung für diese Remote-Adresse gefunden — bitte zuerst verbinden"
                .to_string()
        })?;
    open_saved_at(c, &path)
}
