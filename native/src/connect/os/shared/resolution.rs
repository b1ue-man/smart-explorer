//! Reopen the same filesystem location for GUI jobs and the background daemon.
use std::sync::Arc;

use crate::connect::location::{saved_location, EndpointSpec};
use crate::vfs::BackendHandle;

pub fn resolve_endpoint(endpoint: &str) -> Result<(BackendHandle, String), String> {
    match EndpointSpec::parse(endpoint)? {
        EndpointSpec::Local(path) => {
            if crate::net::is_unc(&path) {
                let connections = crate::creds::load_connections_checked()?;
                if let Some(connection) = connections.iter()
                    .filter(|connection| !connection.protocol.is_url()
                        && unc_is_same_or_below(&path, &connection.root))
                    .max_by_key(|connection| connection.root.len())
                {
                    return super::connector::open_saved_at(connection, &path);
                }
            }
            Ok((Arc::new(crate::vfs::LocalBackend::new(&path)), path))
        }
        EndpointSpec::Drive(path) => super::connector::open_gdrive(&path),
        EndpointSpec::Peer(target, path) => {
            let (_, backend, _) = crate::daemon::open_share_backend(target)?;
            Ok((backend, path))
        }
        EndpointSpec::Saved(endpoint) => {
            let connections = crate::creds::load_connections_checked()?;
            let (connection, path) = saved_location(&connections, &endpoint).ok_or_else(|| {
                "Keine gespeicherte Verbindung für diese Remote-Adresse gefunden — bitte zuerst verbinden".to_string()
            })?;
            super::connector::open_saved_at(connection, &path)
        }
    }
}

fn unc_is_same_or_below(candidate: &str, root: &str) -> bool {
    let candidate = candidate.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let root = root.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    candidate == root || candidate.strip_prefix(&root).is_some_and(|rest| rest.starts_with('\\'))
}

#[cfg(test)]
mod tests {
    use super::unc_is_same_or_below;

    #[test]
    fn unc_saved_root_matching_respects_share_boundary() {
        assert!(unc_is_same_or_below(r"\\server\share\folder", r"\\SERVER\share"));
        assert!(unc_is_same_or_below("//server/share/folder", r"\\server\share\"));
        assert!(!unc_is_same_or_below(r"\\server\share-two\folder", r"\\server\share"));
    }
}
