//! Negotiate literal child names without reinterpreting stored provider paths.
use crate::share::{
    backend::PeerBackend,
    fs_access::FsAccess,
    fs_paths::split_clean,
    peer_stream,
    wire::{FsRequest, FsResponse},
};
use std::io;

pub(super) fn client(
    backend: &PeerBackend,
    parent: &str,
    literal_name: &str,
) -> io::Result<String> {
    validate_literal(literal_name)?;
    if !peer_stream::features(backend, parent)?.literal_children_v1 {
        // The old host has its old literal join semantics. Never use this
        // fallback after a negotiated host rejects or malforms its response.
        return Ok(format!("{}/{}", parent.trim_end_matches('/'), literal_name));
    }
    match backend.request(FsRequest::SyncChildPath {
        parent: parent.into(),
        literal_name: literal_name.into(),
    })? {
        FsResponse::ChildPath { path } => {
            suffix(parent, &path)?;
            Ok(path)
        }
        _ => Err(invalid()),
    }
}

pub(in crate::share) fn host(
    access: &FsAccess,
    parent: &str,
    literal_name: &str,
) -> io::Result<String> {
    validate_literal(literal_name)?;
    access.check_read()?;
    let target = access.resolve(parent)?;
    let provider_child = crate::vfs::sync_child_path(&*target.backend, &target.path, literal_name)?;
    let name = suffix(&target.path, &provider_child)?;
    let virtual_child = format!("{}/{}", parent.trim_end_matches('/'), name);
    let checked = access.resolve(&virtual_child)?;
    if checked.mount_key != target.mount_key
        || checked.path != provider_child
        || checked.backend.namespace_identity() != target.backend.namespace_identity()
    {
        return Err(invalid());
    }
    access.check_read()?;
    Ok(virtual_child)
}

fn suffix<'a>(parent: &str, child: &'a str) -> io::Result<&'a str> {
    let prefix = format!("{}/", parent.trim_end_matches('/'));
    let name = child
        .strip_prefix(&prefix)
        .filter(|name| !name.is_empty())
        .ok_or_else(invalid)?;
    let parts = split_clean(name)?;
    if parts.len() != 1 || parts[0] != name {
        return Err(invalid());
    }
    Ok(name)
}
fn validate_literal(name: &str) -> io::Result<()> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains('/') || name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Kindname ist keine einzelne Komponente",
        ));
    }
    Ok(())
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Share-Kindpfad verlässt seinen freigegebenen Elternpfad",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_literal_children_preserve_encoded_parent_and_distinct_names() {
        let parent = "/Verbindungen/Drive/stored%2520parent";
        for child in ["aux.c", "%2561ux.c", "100%25.pdf", "back%5Cslash"] {
            let path = format!("{parent}/{child}");
            assert_eq!(suffix(parent, &path).unwrap(), child);
        }
        assert!(suffix(parent, "/other/aux.c").is_err());
        assert!(suffix(parent, &format!("{parent}/a/b")).is_err());
        assert!(suffix(parent, &format!("{parent}/../private")).is_err());
        assert!(suffix(parent, &format!("{parent}/raw\\name")).is_err());
        validate_literal("back\\slash").unwrap();
    }
    #[test]
    fn review_task_literal_children_wire_is_read_only_and_legacy_flag_defaults_false() {
        let request = FsRequest::SyncChildPath {
            parent: "/Drive".into(),
            literal_name: "%61ux.c".into(),
        };
        assert!(!request.mutates_filesystem());
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["op"], "sync_child_path");
        assert_eq!(wire["literal_name"], "%61ux.c");
        let legacy: crate::share::wire::FsHostFeatures = serde_json::from_str("{}").unwrap();
        assert!(!legacy.literal_children_v1);
        assert!(crate::share::wire::FsHostFeatures::host().literal_children_v1);
    }
}
