//! Publishing one file of `apply_patch` whole or not at all, as `write`
//! publishes (`file_write.rs`): the content is written to a private local
//! file and published through the `se put` pipeline (stage next to the
//! destination, verify, publish without exposing partial content), bound to
//! the state the patch was computed from (the etag read, or "missing"); the
//! pipeline re-validates that state right before it publishes. A deletion
//! checks the etag right before the file is removed.
use super::super::file_tools::etag;
use crate::cli::link_paths::resolve_destination;
use crate::cli::private_file::PrivateFile;
use crate::cli::target::Target;
use crate::cli::tree_guard::DestinationState;
use crate::cli::tree_observer::TreeObserver;
use crate::cli::tree_ops::TreeRequest;
use std::cell::RefCell;
use std::io::Write;

/// The state a file must still have when it is published or removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Expect {
    /// It did not exist when the patch was computed.
    Missing,
    /// The file with this etag.
    Etag(String),
}

fn changed(path: &str) -> String {
    format!("{path} changed since the patch was computed (etag mismatch); read it again")
}

/// Publishes `bytes` at `target` if it is still in the state `expect`;
/// returns the new etag. Links in the existing part of a local path lead
/// where they point.
pub(super) fn publish(target: &Target, bytes: &[u8], expect: &Expect) -> Result<String, String> {
    let resolved = resolve_destination(target)?;
    let destination = resolved.as_ref().unwrap_or(target);
    let (source, mut file) = PrivateFile::create("smart-explorer-mcp-patch-")?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| format!("cannot create a private staging file: {error}"))?;
    drop(file);
    let refused = RefCell::new(None::<String>);
    let check = |state: &DestinationState| -> Result<(), String> {
        let ok = match (state, expect) {
            (DestinationState::Directory(_), _) => {
                return Err(format!("{} is a directory", target.path))
            }
            (DestinationState::Missing, Expect::Missing) => true,
            (DestinationState::File(meta), Expect::Etag(expected)) => etag(meta) == *expected,
            _ => false,
        };
        if ok {
            Ok(())
        } else {
            *refused.borrow_mut() = Some(changed(&target.path));
            Err("changed".to_string())
        }
    };
    let observer = TreeObserver::default();
    let request = TreeRequest {
        resume: false,
        recursive: false,
        force: true,
        expected_root: None,
        destination_check: Some(&check),
        observer: &observer,
    };
    let outcome = source.publish(&*destination.backend, &destination.path, &request);
    if let Some(message) = refused.borrow_mut().take() {
        return Err(message);
    }
    outcome?;
    let meta = destination
        .backend
        .stat(&destination.path)
        .map_err(|error| format!("written, but cannot read back {}: {error}", target.path))?;
    Ok(etag(&meta))
}

/// Removes the file at `target` if it still has the etag `expected`.
pub(super) fn remove(target: &Target, expected: &str) -> Result<(), String> {
    let meta = target
        .backend
        .stat(&target.path)
        .map_err(|error| format!("cannot read {}: {error}", target.path))?;
    if meta.is_dir || meta.is_symlink || meta.special {
        return Err(format!("{} is not a regular file", target.path));
    }
    if etag(&meta) != expected {
        return Err(changed(&target.path));
    }
    target
        .backend
        .remove_file(&target.path)
        .map_err(|error| format!("cannot delete {}: {error}", target.path))
}
