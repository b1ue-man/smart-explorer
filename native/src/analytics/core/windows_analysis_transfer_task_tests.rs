use super::*;
use crate::analytics::{ScanIssue, ScanStatus, SizeNode};
use std::{cell::RefCell, sync::atomic::Ordering};

#[derive(Clone)]
enum Item {
    Control(AnalysisMessage),
    Data(Vec<u8>),
}

fn encoded(mut outcome: ScanOutcome, files: u64, dirs: u64) -> Vec<Item> {
    let progress = Progress::default();
    progress.files.store(files, Ordering::Relaxed);
    progress.dirs.store(dirs, Ordering::Relaxed);
    progress.bytes.store(
        outcome.tree.as_ref().map_or(0, |node| node.size),
        Ordering::Relaxed,
    );
    let items = RefCell::new(Vec::new());
    send_outcome(
        &mut outcome,
        &progress,
        Some(0),
        |message| {
            items.borrow_mut().push(Item::Control(message));
            Ok(())
        },
        |bytes| {
            items.borrow_mut().push(Item::Data(bytes));
            Ok(())
        },
    )
    .unwrap();
    items.into_inner()
}

fn decode(items: Vec<Item>) -> io::Result<ScanOutcome> {
    let progress = Progress::default();
    let mut receiver = AnalysisReceiver::default();
    for item in items {
        match item {
            Item::Control(message) => {
                if let Some(outcome) = receiver.control(message, &progress)? {
                    return Ok(outcome);
                }
            }
            Item::Data(bytes) => receiver.data(&bytes, &progress)?,
        }
    }
    Err(io::ErrorKind::UnexpectedEof.into())
}

fn partial() -> ScanOutcome {
    let mut outcome = ScanOutcome::complete(SizeNode {
        name: "A".into(),
        size: 7,
        is_dir: true,
        children: vec![SizeNode {
            name: "readable".into(),
            size: 7,
            is_dir: false,
            children: Vec::new(),
        }],
    });
    outcome.status = ScanStatus::Partial;
    outcome.issues.push(ScanIssue {
        path: "/A/denied".into(),
        detail: "Zugriff verweigert".into(),
    });
    outcome.permission_denied = 1;
    outcome.notes.push("Originalhinweis".into());
    outcome
}

#[test]
fn windows_remote_task_analysis_transport_keeps_partial_states_and_rejects_corruption() {
    let outcome = decode(encoded(partial(), 1, 1)).unwrap();
    assert_eq!(outcome.status, ScanStatus::Partial);
    assert_eq!(outcome.permission_denied, 1);
    assert_eq!(outcome.issues[0].path, "/A/denied");
    assert_eq!(outcome.notes, ["Originalhinweis"]);
    assert_eq!(outcome.tree.unwrap().size, 7);
    let mut corrupted = encoded(partial(), 1, 1);
    let Item::Control(AnalysisMessage::Done { sha256 }) = corrupted.last_mut().unwrap() else {
        panic!()
    };
    sha256[0] ^= 1;
    assert_eq!(
        decode(corrupted).err().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
    let mut truncated = encoded(partial(), 1, 1);
    truncated.pop();
    assert_eq!(
        decode(truncated).err().unwrap().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let mut false_complete = encoded(partial(), 1, 1);
    let Item::Control(AnalysisMessage::Ready { report }) = &mut false_complete[0] else {
        panic!()
    };
    report.status = ScanStatus::Complete;
    assert!(decode(false_complete).is_err());
    assert!(AnalysisReceiver::default()
        .data(b"unannounced", &Progress::default())
        .is_err());
    let failed = decode(encoded(ScanOutcome::failed("/A", "read failure"), 0, 0)).unwrap();
    assert_eq!(failed.status, ScanStatus::Failed);
    assert!(failed.tree.is_none());
    let canceled = decode(encoded(ScanOutcome::canceled(), 0, 0)).unwrap();
    assert_eq!(canceled.status, ScanStatus::Canceled);
}

#[test]
fn windows_remote_task_analysis_codec_streams_large_and_deep_local_results() {
    let mut tree = SizeNode {
        name: "root".into(),
        size: 8192,
        is_dir: true,
        children: Vec::new(),
    };
    for index in 0..8192 {
        tree.children.push(SizeNode {
            name: format!("{index}-{}", "n".repeat(64)).into(),
            size: 1,
            is_dir: false,
            children: Vec::new(),
        });
    }
    let items = encoded(ScanOutcome::complete(tree), 8192, 0);
    assert!(
        items
            .iter()
            .filter(|item| matches!(item, Item::Data(_)))
            .count()
            >= 3
    );
    assert!(items
        .iter()
        .all(|item| !matches!(item, Item::Data(bytes) if bytes.len() > crate::agent_proto::CHUNK)));
    assert_eq!(decode(items).unwrap().tree.unwrap().children.len(), 8192);
    let mut deep = SizeNode {
        name: "leaf".into(),
        size: 0,
        is_dir: true,
        children: Vec::new(),
    };
    for _ in 0..600 {
        deep = SizeNode {
            name: "d".into(),
            size: 0,
            is_dir: true,
            children: vec![deep],
        };
    }
    let decoded = decode(encoded(ScanOutcome::complete(deep), 0, 600)).unwrap();
    assert_eq!(decoded.status, ScanStatus::Complete);
}

#[test]
fn windows_remote_task_analysis_progress_retains_stalls_and_actual_counters() {
    for (received, measured, expected) in [
        (None, None, None),
        (Some(0), None, Some(0)),
        (Some(12), Some(50), Some(50)),
    ] {
        let progress = Progress::default();
        progress
            .receive(ScanSnapshot {
                host_scan_ms: received,
                ..Default::default()
            })
            .unwrap();
        let mut reported = None;
        send_outcome(
            &mut ScanOutcome::canceled(),
            &progress,
            measured,
            |message| {
                if let AnalysisMessage::Ready { report } = message {
                    reported = Some(report.progress.host_scan_ms);
                }
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            reported,
            Some(expected),
            "bridges preserve unknown/zero; hosts measure the whole request"
        );
    }
    let progress = Progress::default();
    let state = ScanSnapshot {
        files: 128,
        dirs: 3,
        bytes: 777,
        phase: ScanPhase::Scanning,
        current: "/A/waiting".into(),
        unchanged_ms: 5000,
        source_age_ms: 4000,
        host_scan_ms: Some(0),
        ..Default::default()
    };
    progress.receive(state.clone()).unwrap();
    assert!(progress.snapshot().unchanged_ms >= 5000);
    assert!(progress.remote_report_age().unwrap().as_millis() >= 4000);
    // A fresh IPC heartbeat cannot turn old peer evidence into fresh progress.
    progress.receive(state.clone()).unwrap();
    assert_eq!(progress.snapshot().files, 128);
    assert_eq!(progress.snapshot().host_scan_ms, Some(0));
    assert!(progress.snapshot().unchanged_ms >= 5000);
    let mut backwards = state;
    backwards.files -= 1;
    assert!(progress.receive(backwards).is_err());
    progress.transfer(64, 128);
    assert_eq!(progress.snapshot().transferred, 64);
    progress.set_phase(ScanPhase::Scanning, "/A/next");
    assert_eq!(progress.snapshot().transfer_total, 0);
    progress.set_phase(ScanPhase::Legacy, "/A");
    assert!(progress.snapshot().directories_unreported);
    progress.set_phase(ScanPhase::Scanning, "/A/next");
    assert!(!progress.snapshot().directories_unreported);
    progress.set_node_budget(8);
    let scoped = progress
        .scoped("/real".into(), "/A".into())
        .remote_segment();
    scoped.set_node_budget(4);
    assert_eq!(progress.node_budget(), 4);
    progress.cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        progress.check_cancel().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn review_task_analysis_deflate_reframing_and_receiver_budget() -> io::Result<()> {
    let tree = SizeNode {
        name: "root".into(),
        size: 2048,
        is_dir: true,
        children: (0..2048)
            .map(|i| SizeNode {
                name: format!("{i}-{}", "n".repeat(64)).into(),
                size: 1,
                is_dir: false,
                children: Vec::new(),
            })
            .collect(),
    };
    let progress = Progress::default();
    progress.files.store(2048, Ordering::Relaxed);
    progress.bytes.store(999_999, Ordering::Relaxed);
    let items = RefCell::new(Vec::new());
    send_outcome_with(
        &mut ScanOutcome::complete(tree),
        &progress,
        SendOptions {
            deflate: true,
            host_scan_ms: Some(4),
        },
        |message| {
            items.borrow_mut().push(Item::Control(message));
            Ok(())
        },
        |bytes| {
            items.borrow_mut().push(Item::Data(bytes));
            Ok(())
        },
    )?;
    let items = items.into_inner();
    let Item::Control(ready) = &items[0] else {
        panic!("Ready expected")
    };
    assert_eq!(
        AnalysisReceiver::with_node_budget(2)
            .control(ready.clone(), &Progress::default())
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let received = Progress::default();
    received.receive(ScanSnapshot {
        bytes: 999_999,
        ..Default::default()
    })?;
    let mut receiver = AnalysisReceiver::with_node_budget(2049);
    let mut outcome = None;
    for item in items.clone() {
        match item {
            Item::Control(message) => {
                if let Some(result) = receiver.control(message, &received)? {
                    outcome = Some(result);
                }
            }
            Item::Data(bytes) => {
                for fragment in bytes.chunks(7) {
                    receiver.data(fragment, &received)?;
                }
            }
        }
    }
    let tree = outcome.unwrap().tree.unwrap();
    assert_eq!(
        (tree.size, tree.children.len(), received.snapshot().bytes),
        (2048, 2048, 2048)
    );
    let mut truncated = items;
    if let Some(Item::Data(bytes)) = truncated
        .iter_mut()
        .rev()
        .find(|item| matches!(item, Item::Data(_)))
    {
        bytes.pop();
    }
    assert_eq!(
        decode(truncated).err().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
    Ok(())
}
