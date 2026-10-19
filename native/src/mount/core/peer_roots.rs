//! The mountable roots of a Share device: its shared folders and its
//! connections, moved out of the app (`app/core/mount_peer_roots.rs`, U17)
//! so the drive window of the app and the device tree of the host read a
//! device the same way.
use std::io;

use super::types::BackendRoot;
use crate::vfs::Backend;

/// The folder of a device that holds its connections.
pub const PEER_CONNECTIONS_ROOT: &str = "/Verbindungen";

/// One entry of the root of a device: a shared folder, or the folder of the
/// connections with their names.
pub struct PeerRootListing {
    pub name: String,
    pub connections: Option<Vec<String>>,
}

/// A root to mount and how the drive window names it.
#[derive(Clone)]
pub struct PeerRootChoice {
    pub root: BackendRoot,
    pub label: String,
}

/// Reads the root of a device: every folder that is no link, and the names of
/// the connections below `Verbindungen`.
pub fn list_peer_roots(backend: &dyn Backend) -> io::Result<Vec<PeerRootListing>> {
    let mut listings = Vec::new();
    for entry in backend.list_dir("/")? {
        if !entry.is_dir || entry.is_symlink {
            continue;
        }
        if entry.name == PEER_CONNECTIONS_ROOT.trim_start_matches('/') {
            let Ok(connections) = backend.list_dir(PEER_CONNECTIONS_ROOT) else {
                // Older peers could accidentally advertise a colliding local
                // label here. Do not let one unusable reserved entry hide all
                // other concrete mount roots during discovery.
                continue;
            };
            let connections = connections
                .into_iter()
                .filter(|connection| connection.is_dir && !connection.is_symlink)
                .map(|connection| connection.name)
                .collect();
            listings.push(PeerRootListing {
                name: entry.name,
                connections: Some(connections),
            });
        } else {
            listings.push(PeerRootListing {
                name: entry.name,
                connections: None,
            });
        }
    }
    Ok(listings)
}

/// The roots to offer, in listing order, and last the root `/` „Alle
/// freigegebenen Bereiche“ (read-only). The device tree leaves that one out.
pub fn peer_root_choices(listings: Vec<PeerRootListing>) -> Vec<PeerRootChoice> {
    let mut roots = Vec::new();
    for listing in listings {
        if !safe_virtual_segment(&listing.name) {
            continue;
        }
        if let Some(connections) = listing.connections {
            for connection in connections {
                if safe_virtual_segment(&connection) {
                    push_root(
                        &mut roots,
                        &format!("{PEER_CONNECTIONS_ROOT}/{connection}"),
                        format!("Verbindung: {connection}"),
                    );
                }
            }
        } else {
            push_root(&mut roots, &format!("/{}", listing.name), listing.name);
        }
    }
    push_root(
        &mut roots,
        "/",
        "Alle freigegebenen Bereiche (nur Lesen)".into(),
    );
    roots
}

fn push_root(roots: &mut Vec<PeerRootChoice>, path: &str, label: String) {
    let Ok(root) = BackendRoot::parse(path) else {
        return;
    };
    if roots.iter().any(|choice| choice.root == root) {
        return;
    }
    roots.push(PeerRootChoice { root, label });
}

/// A name a device advertises that may become one path segment.
pub fn safe_virtual_segment(value: &str) -> bool {
    !value.is_empty()
        && value.encode_utf16().count() <= 255
        && !matches!(value, "." | "..")
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
}
