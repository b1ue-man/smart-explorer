//! The pipelined leaf remover of protocol tree deletes (SFTP REMOVE, SMB
//! delete compounds): many removals in flight on one session, one result per
//! leaf in leaf order, nothing started after the first failure.
use futures_util::stream::{self, StreamExt};
use std::future::Future;
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::delete::DeleteTarget;

type FirstFailure = Mutex<Option<(io::ErrorKind, String)>>;

/// Runs `remove` for every leaf with at most `depth` requests in flight and
/// returns one result per leaf, in leaf order. After the first failure no
/// further request starts: the leaves not started yet get that failure's kind
/// and text, so the run stops about where it would have stopped one request
/// at a time (requests already on the wire still finish and count).
pub async fn remove_leaves_pipelined<'a, F, Fut>(
    leaves: &'a [DeleteTarget],
    depth: usize,
    remove: F,
) -> Vec<io::Result<()>>
where
    F: Fn(&'a DeleteTarget) -> Fut,
    Fut: Future<Output = io::Result<()>>,
{
    let failure = FirstFailure::default();
    let failure = &failure;
    stream::iter(leaves)
        .map(|leaf| {
            let request = remove(leaf);
            async move {
                if let Some((kind, text)) = lock(failure).as_ref() {
                    return Err(io::Error::new(*kind, text.clone()));
                }
                let result = request.await;
                if let Err(error) = &result {
                    lock(failure).get_or_insert_with(|| (error.kind(), error.to_string()));
                }
                result
            }
        })
        .buffered(depth.max(1))
        .collect::<Vec<_>>()
        .await
}

fn lock(failure: &FirstFailure) -> MutexGuard<'_, Option<(io::ErrorKind, String)>> {
    failure.lock().unwrap_or_else(PoisonError::into_inner)
}
