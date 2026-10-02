//! Host side of the Share requests added in RV1 (V2): host-side duplicate
//! search, hash walk, folder listings in portions, change watching, the
//! host's trash and finishing stages. The dispatcher (`server_fs.rs`) calls
//! these after it admitted the request: Share active, the session's grant or
//! mount lease, and for writes the export's access, the peer's write right
//! and the protected locations.
//!
//! The streaming entry points own the stream, keep their walk off the QUIC
//! executor and end when the result is complete or the client stops the
//! stream (cancel). The writes are blocking, run on the dispatcher's control
//! pool against a target it resolved and return the reply. A request whose
//! capability this host does not offer (`FsHostFeatures::host`) is answered
//! with `Unsupported`, as an older host would refuse it.
use std::io;

use iroh::endpoint::SendStream;

use super::framing::reply_err;
use super::fs::ResolvedTarget;
use super::fs_access::FsAccess;
use super::session::PeerPrincipal;
use super::wire::{
    FsDuplicateSearch, FsHashWalk, FsListBatch, FsRecycle, FsResponse, FsStageFinish,
    FsSyncFilesystem, FsWatch,
};

/// `duplicate_search_v1`: the host's own duplicate finder below
/// `request.path`, admitted fairly per device; progress, groups and the
/// closing summary travel as `FsResponse::Duplicates`.
pub(in crate::share) async fn serve_duplicate_search(
    mut send: SendStream,
    request: FsDuplicateSearch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let _ = (request, access, principal);
    reply_err(&mut send, not_offered("Die Duplikatsuche auf dem Host")).await
}

/// `hash_walk_v1`: every folder and regular file below `request.path` with
/// size, time and (from `request.min_bytes` on) content hash, and every
/// omission (links, special, unreadable, vanished entries), in portions.
pub(in crate::share) async fn serve_hash_walk(
    mut send: SendStream,
    request: FsHashWalk,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let _ = (request, access, principal);
    reply_err(&mut send, not_offered("Die Prüfsummen-Liste des Hosts")).await
}

/// `list_batches_v1`: the folder `request.path` in name order and bounded
/// portions after `request.cursor`; an unreadable or unrepresentable entry
/// is reported on its own instead of failing the listing.
pub(in crate::share) async fn serve_list_batch(
    mut send: SendStream,
    request: FsListBatch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let _ = (request, access, principal);
    reply_err(&mut send, not_offered("Das Listen in Portionen")).await
}

/// `watch_v1`: a first notice once the export subtree `request.path` is
/// watched, then its change generation (with path hints) or an overflow
/// whenever it changes, until the client closes the stream.
pub(in crate::share) async fn serve_watch(
    mut send: SendStream,
    request: FsWatch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let _ = (request, access, principal);
    reply_err(&mut send, not_offered("Das Beobachten von Freigaben")).await
}

/// `remote_trash_v1`, blocking: moves the file `target` into the host's
/// trash once its size and (when given) its SHA-256 still match `request`;
/// a changed file stays where it is (`FsResponse::Recycle { moved: false }`).
pub(in crate::share) fn serve_recycle(
    target: ResolvedTarget,
    request: FsRecycle,
) -> io::Result<FsResponse> {
    let _ = (target, request);
    Err(not_offered("Der Papierkorb des Hosts"))
}

/// `stage_finish_v1`, blocking: source time, permissions and durability of
/// the client's complete, unpublished stage `target` (only the client's own
/// stage names); answers `FsResponse::StageFinished`.
pub(in crate::share) fn serve_finish_stage(
    target: ResolvedTarget,
    request: FsStageFinish,
) -> io::Result<FsResponse> {
    let _ = (target, request);
    Err(not_offered("Das Abschließen von Zwischendateien"))
}

/// `stage_finish_v1`, blocking: makes the stages finished with deferred
/// durability below `target` durable; answers `FsResponse::Synced`.
pub(in crate::share) fn serve_sync_filesystem(
    target: ResolvedTarget,
    request: FsSyncFilesystem,
) -> io::Result<FsResponse> {
    let _ = (target, request);
    Err(not_offered("Das Sichern abgeschlossener Zwischendateien"))
}

fn not_offered(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} wird von diesem Gerät nicht angeboten"),
    )
}
