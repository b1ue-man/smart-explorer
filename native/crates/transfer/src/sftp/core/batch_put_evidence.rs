//! Upload packets with transfer evidence (G3-03, 2026-10-28;
//! `vfs::put_batch_evidence`). A packet runs in two phases on one pool
//! channel:
//!
//! 1. Every member's stage is created exclusively, written and closed; the
//!    LSTAT of its destination goes out with the WRITEs (no round trip of
//!    its own) and tells whether the destination existed.
//! 2. Back on the caller's thread, outside the runtime, `prepare` writes the
//!    intents of all members as one record (the engine may still look at a
//!    destination the server did not answer for). Only then are the stages
//!    renamed; each RENAME goes out with an LSTAT of its name, whose answer
//!    is the published file's evidence (a regular file of the written size)
//!    or, after a refused RENAME, tells whether the name is taken.
//!
//! Nothing is published before `prepare` succeeded; when it fails, the
//! stages are removed and every member is reported unpublished. The rules of
//! `batch_put.rs` stay: numbered names, an unanswered RENAME is never
//! labeled unpublished, a failure before the RENAME always is.
use super::backend::SftpBackend;
use super::batch_ops::{channel_or_unsupported, window, BatchWire, ChannelWire};
use super::batch_put::{judged, open_stage, read_member, stage_written, unknown};
use super::stage_evidence::regular_evidence;
use crate::transfer::engine_names::next_numbered;
use crate::vfs::{unpublished_error, BatchPut, EvidencePutOutcome, PreparePut, VfsResult};
use futures_util::future::{join, join_all, FutureExt, LocalBoxFuture};
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::io::{self, Read};

/// A member after the first phase.
enum Staged {
    /// Written and closed under `stage`; `existed`: what the LSTAT of the
    /// destination answered (`None`: no answer).
    Ready {
        stage: String,
        existed: Option<bool>,
    },
    Failed(io::Error),
}

type Running<'a> = FuturesUnordered<LocalBoxFuture<'a, (usize, Staged)>>;

/// `vfs::put_batch_evidence` of the SFTP backend: `Unsupported` before
/// anything is read when the packet cannot run on a pool channel.
pub(super) fn run(
    backend: &SftpBackend,
    entries: &[BatchPut],
    data: &mut dyn Read,
    prepare: &mut PreparePut<'_>,
) -> VfsResult<Vec<EvidencePutOutcome>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    if entries.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Ein Paket braucht mindestens zwei Dateien",
        ));
    }
    let lease = channel_or_unsupported(backend.pool.lease(backend)?)?;
    let wire = ChannelWire::new(lease.channel().clone(), backend.connection.clone());
    let staged = backend.rt.block_on(stage_all(&wire, entries, data));
    let existed: Vec<Option<bool>> = staged
        .iter()
        .map(|member| match member {
            Staged::Ready { existed, .. } => *existed,
            Staged::Failed(_) => None,
        })
        .collect();
    // Outside the runtime: the intents are durable before any RENAME.
    let outcomes = match prepare(0, existed.as_slice()) {
        Ok(()) => backend.rt.block_on(publish_all(&wire, entries, staged)),
        Err(error) => {
            backend.rt.block_on(remove_all(&wire, &staged));
            staged
                .into_iter()
                .map(|member| {
                    EvidencePutOutcome::Failed(match member {
                        Staged::Failed(failure) => failure,
                        Staged::Ready { .. } => unpublished_error(
                            error.kind(),
                            format!("Paketpublikation nicht vorbereitet: {error}"),
                        ),
                    })
                })
                .collect()
        }
    };
    drop(lease);
    Ok(outcomes)
}

/// Phase one for every entry, in entry order; at most `window` members hold
/// a handle (and their bytes) at once.
async fn stage_all(wire: &dyn BatchWire, entries: &[BatchPut], data: &mut dyn Read) -> Vec<Staged> {
    let window = window(wire, entries.len());
    let mut running: Running<'_> = FuturesUnordered::new();
    let mut done: Vec<Option<Staged>> = entries.iter().map(|_| None).collect();
    for (index, entry) in entries.iter().enumerate() {
        while running.len() >= window {
            match running.next().await {
                Some((at, staged)) => store(&mut done, at, staged),
                None => break,
            }
        }
        match read_member(data, entry.size) {
            Ok(bytes) => {
                let member = stage_member(wire, &entry.path, bytes, entry.mtime_ms);
                running.push(async move { (index, member.await) }.boxed_local());
                // Its first request goes out now, not after the packet is read.
                if let Some(Some((at, staged))) = running.next().now_or_never() {
                    store(&mut done, at, staged);
                }
            }
            Err(error) => {
                // This entry and every later one never started.
                store(&mut done, index, Staged::Failed(error));
                break;
            }
        }
    }
    while let Some((at, staged)) = running.next().await {
        store(&mut done, at, staged);
    }
    done.into_iter()
        .map(|staged| {
            staged.unwrap_or_else(|| {
                Staged::Failed(unpublished_error(
                    io::ErrorKind::Interrupted,
                    "Paket abgebrochen",
                ))
            })
        })
        .collect()
}

fn store(done: &mut [Option<Staged>], index: usize, staged: Staged) {
    if let Some(slot) = done.get_mut(index) {
        *slot = Some(staged);
    }
}

/// Stage, WRITEs and CLOSE, with the LSTAT of the destination behind them.
async fn stage_member(
    wire: &dyn BatchWire,
    destination: &str,
    bytes: Vec<u8>,
    mtime_ms: Option<i64>,
) -> Staged {
    let (stage, handle) = match open_stage(wire, destination).await {
        Ok(opened) => opened,
        Err(error) => return Staged::Failed(error),
    };
    let (written, existed) = join(
        stage_written(wire, &handle, &bytes, mtime_ms),
        wire.exists(destination),
    )
    .await;
    if let Err(failure) = written {
        let _ = wire.remove(&stage).await;
        return Staged::Failed(judged(failure, &stage));
    }
    Staged::Ready {
        stage,
        existed: existed.ok(),
    }
}

/// Phase two: every RENAME at once.
async fn publish_all(
    wire: &dyn BatchWire,
    entries: &[BatchPut],
    staged: Vec<Staged>,
) -> Vec<EvidencePutOutcome> {
    let members = entries
        .iter()
        .zip(staged)
        .map(|(entry, member)| async move {
            match member {
                Staged::Ready { stage, .. } => publish(wire, &stage, &entry.path, entry.size).await,
                Staged::Failed(error) => EvidencePutOutcome::Failed(error),
            }
        });
    join_all(members).await
}

/// Removes the stages of a packet whose intents could not be written.
async fn remove_all(wire: &dyn BatchWire, staged: &[Staged]) {
    let removals = staged.iter().filter_map(|member| match member {
        Staged::Ready { stage, .. } => Some(wire.remove(stage)),
        Staged::Failed(_) => None,
    });
    join_all(removals).await;
}

/// Renames the stage to `destination` or the next free "name (n)"; the
/// LSTAT sent with each RENAME is the evidence or the taken-check.
async fn publish(
    wire: &dyn BatchWire,
    stage: &str,
    destination: &str,
    size: u64,
) -> EvidencePutOutcome {
    let (parent, name) = match destination.rfind('/') {
        Some(index) => destination.split_at(index + 1),
        None => ("", destination),
    };
    let numbered = next_numbered(name).map(|numbered| format!("{parent}{numbered}"));
    for candidate in std::iter::once(destination.to_string()).chain(numbered) {
        let (renamed, looked) = join(wire.rename(stage, &candidate), wire.stat(&candidate)).await;
        let failure = match renamed {
            Ok(()) => {
                let evidence = looked
                    .ok()
                    .and_then(|attrs| regular_evidence(&candidate, &attrs))
                    .filter(|evidence| evidence.size == size);
                return EvidencePutOutcome::Published {
                    path: candidate,
                    evidence,
                };
            }
            Err(failure) => failure,
        };
        if !failure.answered {
            return EvidencePutOutcome::Failed(unknown(failure, stage));
        }
        // Something of any kind holds the name: the next one.
        if looked.is_ok() {
            continue;
        }
        let _ = wire.remove(stage).await;
        return EvidencePutOutcome::Failed(judged(failure, stage));
    }
    let _ = wire.remove(stage).await;
    EvidencePutOutcome::Failed(unpublished_error(
        io::ErrorKind::AlreadyExists,
        format!("Kein freier Name für „{destination}“"),
    ))
}
