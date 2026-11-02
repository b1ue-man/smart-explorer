//! Loopback coverage of the analysis job id (`analysis_reattach_v1`) between
//! two real Iroh nodes: the id this client sends and the base64url id that
//! clients 0.5.170 to 0.5.177 sent both reach the host's analysis and
//! duplicate workers; a malformed id is still refused.
use std::fs;
use std::io;

use super::CopyPastePeerFixture;
use crate::analytics::{Progress, ReclaimProgress, ScanOutcome, ScanStatus};
use crate::share::wire::{FsDuplicateSearch, FsStorageAnalysis};

const FILE_BYTES: usize = 2 * 1024 * 1024;

/// Export `A` with one duplicate pair: `/A/one.bin` and `/A/sub/two.bin`.
fn fixture_with_duplicates() -> io::Result<CopyPastePeerFixture> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let payload = vec![7u8; FILE_BYTES];
    fs::create_dir(fixture.root_a.join("sub"))?;
    fs::write(fixture.root_a.join("one.bin"), &payload)?;
    fs::write(fixture.root_a.join("sub").join("two.bin"), &payload)?;
    Ok(fixture)
}

fn assert_complete(outcome: &ScanOutcome) {
    let issues = outcome
        .issues
        .iter()
        .map(|issue| issue.detail.as_str())
        .collect::<Vec<_>>();
    assert_eq!(outcome.status, ScanStatus::Complete, "{issues:?}");
    let tree = outcome.tree.as_ref().expect("analysis tree");
    assert!(
        tree.size >= 2 * FILE_BYTES as u64,
        "tree size {}",
        tree.size
    );
}

fn assert_one_pair(report: &crate::analytics::DuplicateReport) {
    assert_eq!(report.groups.len(), 1, "{report:?}");
    let mut names = report.groups[0]
        .items
        .iter()
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["one.bin", "two.bin"], "{report:?}");
}

#[test]
fn analysis_id_task_remote_analysis_and_duplicates_reach_the_host() -> io::Result<()> {
    let fixture = fixture_with_duplicates()?;
    let outcome =
        crate::share::peer_storage_analysis::scan(&fixture.peer, "/A", &Progress::default())?
            .expect("the host analyses its own export");
    assert_complete(&outcome);
    let report =
        crate::share::peer_duplicates::find(&fixture.peer, "/A", 1, &ReclaimProgress::default())?
            .expect("the host searches its own duplicates");
    assert_one_pair(&report);
    Ok(())
}

#[test]
fn analysis_id_task_host_accepts_ids_of_released_clients() -> io::Result<()> {
    let fixture = fixture_with_duplicates()?;
    let released = || crate::share::core::random_token(16).map_err(io::Error::other);
    let progress = Progress::default();
    let analysis = |request_id: String| FsStorageAnalysis {
        path: "/A".into(),
        node_budget: Some(progress.node_budget()),
        compress: false,
        request_id: Some(request_id),
    };
    let outcome = crate::share::peer_storage_analysis::request_with(
        &fixture.peer,
        analysis(released()?),
        false,
        &progress,
    )?;
    assert_complete(&outcome);
    let report = crate::share::peer_duplicates::receive(
        &fixture.peer,
        FsDuplicateSearch {
            path: "/A".into(),
            min_bytes: 1,
            request_id: Some(released()?),
        },
        &ReclaimProgress::default(),
    )?;
    assert_one_pair(&report);

    let refused = crate::share::peer_storage_analysis::request_with(
        &fixture.peer,
        analysis("../A/one.bin/x/y".into()),
        false,
        &Progress::default(),
    );
    match refused {
        Err(error) => assert!(
            error.to_string().contains("Ungültige Analyse-Auftrags-ID"),
            "{error}"
        ),
        Ok(_) => panic!("a path-like id must be refused"),
    }
    Ok(())
}
