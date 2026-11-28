//! `content_search_v1` client: the host searches the files it holds and
//! streams only the hits; they reach `tx` with channel backpressure. A host
//! without the feature is never asked (`Ok(None)`: the caller reads the
//! files through the backend). The closing totals must equal what the
//! portions carried, so a cut stream never passes as a complete search.
use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary};
use crate::share::{
    backend::PeerBackend,
    framing,
    fs_response::{FsContentEvent, FsContentMessage, FsContentQuery, FsContentSearch},
    io_deadline, peer_stream,
    wire::{Ctrl, FsRequest, FsResponse},
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// Whether the host offers its own content search.
pub(in crate::share) fn supported(backend: &PeerBackend) -> bool {
    peer_stream::features(backend, "/").is_ok_and(|features| features.content_search_v1)
}

pub(in crate::share) fn search(
    backend: &PeerBackend,
    root: &str,
    query: &ContentQuery,
    tx: crossbeam_channel::Sender<ContentEvent>,
    cancel: &AtomicBool,
) -> io::Result<Option<ContentSummary>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let features = peer_stream::features(backend, root)?;
    // A host without `content_search_ignore_v1` would drop the field and
    // search ignored files: the search then reads through the backend.
    if !features.content_search_v1
        || (query.respect_ignore_files && !features.content_search_ignore_v1)
    {
        return Ok(None);
    }
    let endpoint = backend.current_endpoint()?;
    // A person's search is presence until it ends (E4).
    let _presence = backend.metadata_presence(&endpoint);
    let origin = backend.effective_origin();
    let mut opened = backend.node.open_stream(&endpoint, &backend.identity)?;
    let lease = backend.mount_lease_token()?;
    let req = FsRequest::ContentSearch(FsContentSearch {
        path: root.into(),
        query: FsContentQuery::from(query),
    });
    let result = backend.node.block_on_route(opened.generation, async {
        io_deadline::run(
            "Share content search request",
            framing::send_fs_ctrl(&mut opened.send, &Ctrl::Fs { req, lease }, origin),
        )
        .await?;
        let mut counted = (0u64, 0u64);
        loop {
            let message = match peer_stream::response(&mut opened.recv, cancel).await? {
                FsResponse::ContentSearch { message } => message,
                _ => return Err(peer_stream::invalid("Unerwartete Antwort der Inhaltssuche")),
            };
            match message {
                FsContentMessage::Progress { files, bytes, hits } => {
                    // Totals only: a full channel skips one.
                    let _ = tx.try_send(ContentEvent::Progress { files, bytes, hits });
                }
                FsContentMessage::Events { events } => {
                    for event in events {
                        count(&event, &mut counted)?;
                        let item = ContentEvent::from(event);
                        crate::share::peer_hash_walk::publish(&tx, cancel, item).await?;
                    }
                }
                FsContentMessage::Done { summary } => {
                    if (summary.hits, summary.skipped) != counted {
                        return Err(peer_stream::invalid("Summe der Inhaltssuche stimmt nicht"));
                    }
                    return Ok(Some(ContentSummary::from(summary)));
                }
            }
        }
    });
    if let Err(error) = &result {
        peer_stream::abort(
            &mut opened.send,
            &mut opened.recv,
            cancel.load(Ordering::Relaxed),
        );
        if peer_stream::transport(error) {
            let _ = backend
                .node
                .invalidate_outgoing_session(&opened.session_key, opened.generation);
        }
    }
    result
}

/// Checks one event of the host and counts its hit lines and skips.
fn count(event: &FsContentEvent, counted: &mut (u64, u64)) -> io::Result<()> {
    let rel = event.rel();
    if !(rel.is_empty() || peer_stream::relative(rel)) {
        return Err(peer_stream::invalid("Treffer außerhalb der Suchwurzel"));
    }
    match event {
        FsContentEvent::Hits { lines, .. } => {
            if rel.is_empty() || !lines.iter().all(|line| line.well_formed()) {
                return Err(peer_stream::invalid("Ungültiger Treffer der Inhaltssuche"));
            }
            let hits = lines.iter().filter(|line| !line.context).count() as u64;
            counted.0 = counted.0.saturating_add(hits);
        }
        FsContentEvent::Skipped { .. } => counted.1 = counted.1.saturating_add(1),
    }
    Ok(())
}
