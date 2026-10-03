//! Empty directories share apply's omission, cancellation and durability rules.
use std::io;
use std::sync::atomic::AtomicBool;
use super::checkpoint::ApplyScope;
use super::completion::{CompletedAction, CompletedKind, DirAction};
use super::incremental::SyncEndpoints;
use super::types::{BisyncOptions, PairSide};

pub(super) fn run(action: &DirAction, endpoints: SyncEndpoints<'_>, opts: BisyncOptions,
    scope: &ApplyScope<'_>, cancel: &AtomicBool) -> io::Result<()> {
    super::apply_transaction::gate(cancel, Some(scope.sink))?;
    let side = action.side();
    let (backend, root) = match side { PairSide::A => (endpoints.a, endpoints.root_a),
        PairSide::B => (endpoints.b, endpoints.root_b) };
    let rel = scope.spellings.side_rel(action.rel(), side);
    super::apply_boundary::guard(backend, root, rel, opts.cross_mounts)?;
    super::apply_boundary::target(backend, root, rel, None)?;
    let path = crate::vfs::sync_path(backend, root, rel)?;
    let kind = match action {
        DirAction::Create { .. } => {
            match backend.stat(&path) {
                Ok(meta) if meta.is_dir && !meta.is_symlink && !meta.special => {},
                Ok(_) => return Err(super::apply_guard::drift("planned directory changed type")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    super::apply_transaction::gate(cancel, Some(scope.sink))?;
                    backend.mkdir_all(&path)?;
                }
                Err(error) => return Err(error),
            }
            CompletedKind::DirCreated { side }
        }
        DirAction::Remove { .. } => {
            let meta = match backend.stat(&path) {
                Ok(meta) => meta,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    super::apply_stage::require_durable(super::apply_stage::namespace(backend, &path)?)?;
                    scope.sink.completed(CompletedAction { rel: action.rel().to_string(),
                        kind: CompletedKind::DirRemoved { side }, src_sig: None, dst_sig: None, durable: true });
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            if !meta.is_dir || meta.is_symlink || meta.special {
                return Err(super::apply_guard::drift("planned empty directory changed type"));
            }
            let listing = crate::vfs::list_dir_tolerant(backend, &path)?;
            if !listing.entries.is_empty() || !listing.omitted.is_empty() {
                return Err(super::apply_guard::drift("directory is no longer empty"));
            }
            super::apply_transaction::gate(cancel, Some(scope.sink))?;
            super::apply_boundary::guard(backend, root, rel, opts.cross_mounts)?;
            let (peer, peer_root) = if side == PairSide::A { (endpoints.b,endpoints.root_b) } else { (endpoints.a,endpoints.root_a) };
            let keys = super::KeyPolicy::for_pair(backend.case_sensitive_paths(root),peer.case_sensitive_paths(peer_root));
            if !super::apply_boundary::normalized_missing(peer,peer_root,
                scope.spellings.side_rel(action.rel(),side.other()),keys,opts.cross_mounts,cancel)? {
                return Err(super::apply_guard::drift("source directory appeared before removal"));
            }
            backend.remove_dir(&path)?;
            CompletedKind::DirRemoved { side }
        }
    };
    super::apply_stage::require_durable(super::apply_stage::namespace(backend, &path)?)?;
    scope.sink.completed(CompletedAction { rel: action.rel().to_string(), kind,
        src_sig: None, dst_sig: None, durable: true });
    Ok(())
}
