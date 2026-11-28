//! `content_search_v1` between two real Iroh nodes on loopback (plan
//! 2026-11-27 section B): the host searches its export and only the hits
//! reach the client, with totals that match them; a host without the
//! feature is never asked (`Ok(None)`, the caller then reads the files).
use std::fs;
use std::io;
use std::sync::atomic::AtomicBool;

use super::CopyPastePeerFixture;
use crate::agent_proto::{ContentEvent, ContentQuery};
use crate::share::peer_content_search;
use crate::vfs::Backend;

fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn content_search_task_share_host_searches_and_older_hosts_are_not_asked() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fs::create_dir(fixture.root_a.join("sub"))?;
    fs::write(
        fixture.root_a.join("sub").join("hit.txt"),
        b"alpha\nthe needle\n",
    )?;
    fs::write(fixture.root_a.join("miss.txt"), b"nothing")?;
    let query = ContentQuery::text("needle");

    let (tx, rx) = crossbeam_channel::unbounded();
    let summary = peer_content_search::search(&fixture.peer, "/A", &query, tx, &idle())?
        .expect("the host offers content_search_v1");
    let hits: Vec<(String, Vec<(u64, String)>)> = rx
        .try_iter()
        .filter_map(|event| match event {
            ContentEvent::Hits { rel, lines, .. } => Some((
                rel,
                lines
                    .into_iter()
                    .filter(|line| !line.context)
                    .map(|line| (line.line, line.text))
                    .collect(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        hits,
        vec![(
            "sub/hit.txt".to_string(),
            vec![(2, "the needle".to_string())]
        )]
    );
    assert_eq!(summary.hits, 1);
    assert_eq!(summary.files_matched, 1);
    assert_eq!(summary.files_searched, 2);
    assert!(!summary.limited);
    assert!(fixture.peer.supports_content_search());

    // A host that does not offer `content_search_v1` is never asked.
    if let Ok(mut slot) = fixture.peer.transfer.features.lock() {
        if let Some((_, features)) = slot.as_mut() {
            features.content_search_v1 = false;
        }
    }
    let (tx, rx) = crossbeam_channel::unbounded();
    assert_eq!(
        peer_content_search::search(&fixture.peer, "/A", &query, tx, &idle())?,
        None
    );
    assert!(rx.try_recv().is_err());
    assert!(!fixture.peer.supports_content_search());
    Ok(())
}
