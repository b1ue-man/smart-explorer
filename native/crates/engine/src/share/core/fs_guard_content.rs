//! A content search behind the export guard: the served backend searches
//! (on its own server where it can), never in the Share-private folders and
//! files (`fs_policy::private_name`), and every event whose path the policy
//! hides is left out of the stream and out of the totals.
use super::{stream, GuardedBackend};
use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary};
use crate::vfs::VfsResult;
use std::sync::atomic::AtomicBool;

/// Filter words that keep the served backend out of the Share-private
/// names (`.se-versions`, `.held.se-recycle-<16 hex>`) as files and as
/// folders of every level (`ContentQuery::filters`, Notepad++ syntax).
const PRIVATE_FILTERS: &str = "!.se-versions !+\\.se-versions \
     !.held.se-recycle-???????????????? !+\\.held.se-recycle-????????????????";

pub(super) fn search(
    guard: &GuardedBackend,
    root: &str,
    query: &ContentQuery,
    tx: crossbeam_channel::Sender<ContentEvent>,
    cancel: &AtomicBool,
) -> VfsResult<Option<ContentSummary>> {
    guard.read(root)?;
    let query = without_private(query);
    let mut left_out = ContentSummary::default();
    let summary = stream::forward(
        guard.authority.as_ref(),
        &guard.policy,
        root,
        tx,
        cancel,
        relative,
        |event| tally(&mut left_out, event),
        |output, cancelled| guard.inner.content_search(root, &query, output, cancelled),
    )?;
    Ok(summary.map(|summary| ContentSummary {
        files_matched: summary.files_matched.saturating_sub(left_out.files_matched),
        hits: summary.hits.saturating_sub(left_out.hits),
        skipped: summary.skipped.saturating_sub(left_out.skipped),
        ..summary
    }))
}

/// `query` with the private names excluded.
pub(super) fn without_private(query: &ContentQuery) -> ContentQuery {
    let mut query = query.clone();
    query.filters = if query.filters.trim().is_empty() {
        PRIVATE_FILTERS.to_string()
    } else {
        format!("{} {PRIVATE_FILTERS}", query.filters)
    };
    query
}

fn relative(event: &ContentEvent) -> &str {
    match event {
        ContentEvent::Hits { rel, .. } | ContentEvent::Skipped { rel, .. } => rel,
        ContentEvent::Progress { .. } => "",
    }
}

/// What a hidden event would have added to the totals.
fn tally(left_out: &mut ContentSummary, event: &ContentEvent) {
    match event {
        ContentEvent::Hits { lines, last, .. } => {
            let hits = lines.iter().filter(|line| !line.context).count() as u64;
            left_out.hits = left_out.hits.saturating_add(hits);
            if *last {
                left_out.files_matched = left_out.files_matched.saturating_add(1);
            }
        }
        ContentEvent::Skipped { .. } => left_out.skipped = left_out.skipped.saturating_add(1),
        ContentEvent::Progress { .. } => {}
    }
}
