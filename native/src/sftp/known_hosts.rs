use std::io::Write;
use std::path::PathBuf;

fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn known_hosts_path() -> PathBuf {
    app_data_dir().join("known_hosts_sftp.txt")
}

/// TOFU: accept a matching or first-seen key (persisting it), reject a changed
/// key. Returns whether russh should accept the handshake.
pub(super) fn known_hosts_accept(host: &str, port: u16, key: &russh::keys::PublicKey) -> bool {
    let fp = key.fingerprint(Default::default()).to_string();
    let id = format!("{host}:{port}");
    let path = known_hosts_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            let mut it = line.split_whitespace();
            if let (Some(h), Some(f)) = (it.next(), it.next()) {
                if h == id {
                    return f == fp; // known host → must match
                }
            }
        }
    }
    // unknown → accept and persist
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{id} {fp}");
    }
    true
}
