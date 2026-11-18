//! Occupied bytes across the analysis transfer: the `allocated` record
//! format, a host or receiver without it, and the aggregate fit.
use super::*;
use crate::analytics::{fit_tree, SizeNode};

fn node(name: &str, size: u64, allocated: u64) -> SizeNode {
    SizeNode {
        name: name.into(),
        size,
        allocated,
        is_dir: false,
        children: Vec::new(),
    }
}

fn tree() -> SizeNode {
    let children = vec![
        node("placeholder.iso", 5_000_000_000, 0),
        node("compressed.log", 1_000_000, 300_000),
        node("plain.bin", 10, 4096),
    ];
    SizeNode {
        name: "/".into(),
        size: children.iter().map(|child| child.size).sum(),
        allocated: children.iter().map(|child| child.allocated).sum(),
        is_dir: true,
        children,
    }
}

/// Control messages as JSON text (the wire form) and the data frames.
fn send(mut outcome: ScanOutcome, records: bool) -> io::Result<(Vec<String>, Vec<Vec<u8>>)> {
    let progress = Progress::default();
    let (mut controls, mut data) = (Vec::new(), Vec::new());
    send_outcome_allocated(
        &mut outcome,
        &progress,
        SendOptions {
            host_scan_ms: Some(1),
            deflate: true,
        },
        records,
        |message| {
            controls.push(serde_json::to_string(&message).map_err(io::Error::other)?);
            Ok(())
        },
        |bytes| {
            data.push(bytes);
            Ok(())
        },
    )?;
    Ok((controls, data))
}

fn receive(controls: &[String], data: &[Vec<u8>]) -> io::Result<(ScanOutcome, Progress)> {
    let progress = Progress::default();
    let mut receiver = AnalysisReceiver::default();
    let ready: AnalysisMessage = serde_json::from_str(&controls[0]).map_err(io::Error::other)?;
    assert!(receiver.control(ready, &progress)?.is_none());
    for bytes in data {
        receiver.data(bytes, &progress)?;
    }
    let done: AnalysisMessage = serde_json::from_str(&controls[1]).map_err(io::Error::other)?;
    let outcome = receiver
        .control(done, &progress)?
        .ok_or_else(|| io::Error::other("kein Ergebnis"))?;
    Ok((outcome, progress))
}

#[test]
fn belegt_task_transfer_carries_allocation_and_old_peers_stay_unknown() -> io::Result<()> {
    let known = || {
        let mut outcome = ScanOutcome::complete(tree());
        outcome.allocated_known = true;
        outcome
    };
    // Both sides with the format: every node keeps its occupied bytes.
    let (controls, data) = send(known(), true)?;
    let (outcome, progress) = receive(&controls, &data)?;
    assert!(outcome.allocated_known);
    let received = outcome.tree.expect("tree");
    assert_eq!(received.allocated, 304_096);
    assert_eq!(received.size, 5_001_000_010);
    let occupied: Vec<u64> = received.children.iter().map(|n| n.allocated).collect();
    assert_eq!(occupied, [0, 300_000, 4096]);
    assert_eq!(progress.snapshot().allocated, 304_096);

    // A receiver that did not ask: the old 17-byte records, no flags.
    let (controls, data) = send(known(), false)?;
    assert!(!controls[0].contains("allocated_records"));
    let (outcome, _) = receive(&controls, &data)?;
    assert!(!outcome.allocated_known);
    let received = outcome.tree.expect("tree");
    assert!(received.children.iter().all(|n| n.allocated == n.size));
    assert_eq!(received.allocated, received.size);

    // A host before the format: its report and progress lack every new key.
    let (mut controls, data) = send(known(), false)?;
    let mut ready: serde_json::Value =
        serde_json::from_str(&controls[0]).map_err(io::Error::other)?;
    let report = &mut ready["Ready"]["report"];
    if let Some(report) = report.as_object_mut() {
        report.remove("allocated_known");
    }
    if let Some(progress) = report["progress"].as_object_mut() {
        progress.remove("allocated");
    }
    controls[0] = ready.to_string();
    assert!(!controls[0].contains("allocated"));
    let (outcome, progress) = receive(&controls, &data)?;
    assert!(!outcome.allocated_known);
    assert_eq!(outcome.tree.expect("tree").allocated, 5_001_000_010);
    assert_eq!(progress.snapshot().allocated, 5_001_000_010);
    Ok(())
}

#[test]
fn belegt_task_decoder_rejects_contradicting_folder_occupancy() {
    let mut broken = tree();
    broken.allocated += 1;
    let progress = Progress::default();
    let shape = tree_transfer::shape_with(Some(&broken), &progress, true).expect("shape");
    let mut raw = Vec::new();
    tree_transfer::encode_with(Some(broken), &progress, true, |bytes| {
        raw.extend(bytes);
        Ok(())
    })
    .expect("encode");
    assert_eq!(shape.bytes, raw.len() as u64);
    let mut decoder = TreeDecoder::default();
    decoder.set_shape(shape);
    decoder.set_allocated(true);
    let error = decoder.push(&raw, &progress).expect_err("contradiction");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn belegt_task_fit_tree_aggregates_keep_both_sums() -> io::Result<()> {
    let mut root = tree();
    let progress = Progress::default();
    fit_tree(&mut root, 3, &progress)?;
    assert_eq!(root.children.len(), 2);
    let sum = |pick: fn(&SizeNode) -> u64| root.children.iter().map(pick).sum::<u64>();
    assert_eq!(sum(|n| n.size), root.size);
    assert_eq!(sum(|n| n.allocated), root.allocated);
    assert_eq!(root.children[1].allocated, 304_096);
    Ok(())
}
