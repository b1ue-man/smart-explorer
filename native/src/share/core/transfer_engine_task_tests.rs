//! Loopback coverage of Share transfer v1 uploads (Block B1): batches,
//! numbering, authorization and abort per entry, and the lost-reply status.
use std::fs;
use std::io;
use std::time::{Duration, Instant};

use super::transfer_engine_task_support::{
    describe, get, pattern, put, stage_names, stages_after_cleanup, FailingSource, RecordingSink,
};
use super::CopyPastePeerFixture;
use crate::share::framing::{recv_resp_wire, send_ctrl, send_tagged, TAG_DATA};
use crate::share::io_deadline;
use crate::share::wire::{
    Ctrl, FsBatchPut, FsBatchStatus, FsRequest, FsResponse, BATCH_MAX_BYTES, BATCH_MAX_FILES,
};
use crate::vfs::BatchPutOutcome;

#[test]
fn transfer_engine_task_share_put_batch_publishes_and_numbers() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &fixture.backend;
    let inbox = fixture.root_a.join("inbox");
    fs::create_dir(&inbox)?;
    fs::write(inbox.join("taken.txt"), b"existing")?;
    let limits = backend
        .batch_limits("/A/inbox")
        .expect("a transfer v1 host offers batches");
    assert_eq!(limits.max_files, BATCH_MAX_FILES as usize);
    assert_eq!(limits.max_bytes, BATCH_MAX_BYTES);

    let entries = [
        put("/A/inbox/one.txt", 3),
        put("/A/inbox/empty.bin", 0),
        put("/A/inbox/taken.txt", 5),
        put("/A/inbox/Gr\u{fc}\u{df}e.txt", 4),
    ];
    let mut data: &[u8] = b"onefreshdata";
    let outcomes = backend.put_batch(&entries, &mut data)?;
    assert_eq!(
        describe(&outcomes),
        [
            "/A/inbox/one.txt",
            "/A/inbox/empty.bin",
            "/A/inbox/taken (2).txt",
            "/A/inbox/Gr\u{fc}\u{df}e.txt",
        ]
    );
    assert!(data.is_empty(), "every byte was consumed");
    assert_eq!(fs::read(inbox.join("one.txt"))?, b"one");
    assert_eq!(fs::read(inbox.join("empty.bin"))?, b"");
    assert_eq!(
        fs::read(inbox.join("taken.txt"))?,
        b"existing",
        "never replaced"
    );
    assert_eq!(fs::read(inbox.join("taken (2).txt"))?, b"fresh");
    assert_eq!(fs::read(inbox.join("Gr\u{fc}\u{df}e.txt"))?, b"data");
    assert!(stage_names(&inbox)?.is_empty());

    // Several data frames, a name taken twice and a second export.
    let payload = pattern(600 * 1024, 251);
    let mut bytes = payload.clone();
    bytes.extend_from_slice(b"ok");
    let second = [
        put("/A/inbox/taken.txt", payload.len() as u64),
        put("/B/other.bin", 2),
    ];
    let outcomes = backend.put_batch(&second, &mut bytes.as_slice())?;
    assert_eq!(
        describe(&outcomes),
        ["/A/inbox/taken (3).txt", "/B/other.bin"]
    );
    assert_eq!(fs::read(inbox.join("taken (3).txt"))?, payload);
    assert_eq!(fs::read(fixture.root_b.join("other.bin"))?, b"ok");
    assert!(stage_names(&inbox)?.is_empty());
    Ok(())
}

#[test]
fn transfer_engine_task_share_batches_authorize_every_entry() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &fixture.backend;
    let entries = [
        put("/A/ok.txt", 2),
        put("/Unexported/x.txt", 2),
        put("/A/../escape.txt", 2),
        put("/B/fine.txt", 2),
        put("/A", 2),
    ];
    let outcomes = backend.put_batch(&entries, &mut &b"okxxyyfizz"[..])?;
    let described = describe(&outcomes);
    assert_eq!(described[0], "/A/ok.txt");
    assert!(described[1].starts_with("failed"), "{described:?}");
    assert!(described[2].starts_with("failed"), "{described:?}");
    assert_eq!(described[3], "/B/fine.txt");
    assert!(described[4].starts_with("failed"), "{described:?}");
    assert_eq!(fs::read(fixture.root_a.join("ok.txt"))?, b"ok");
    assert_eq!(fs::read(fixture.root_b.join("fine.txt"))?, b"fi");
    if let Some(parent) = fixture.root_a.parent() {
        assert!(!parent.join("escape.txt").exists());
        assert!(
            stage_names(parent)?.is_empty(),
            "no stage beside the exports"
        );
    }
    assert!(stage_names(&fixture.root_a)?.is_empty());
    assert!(stage_names(&fixture.root_b)?.is_empty());

    // A mounted client: every entry must lie below its lease root.
    backend.mount_path_capabilities("/A")?;
    let outcomes = backend.put_batch(
        &[put("/A/leased.txt", 1), put("/B/outside.txt", 1)],
        &mut &b"lo"[..],
    )?;
    assert_eq!(
        describe(&outcomes),
        ["/A/leased.txt", "failed PermissionDenied"]
    );
    assert!(!fixture.root_b.join("outside.txt").exists());
    let mut sink = RecordingSink::default();
    backend.get_batch(&[get("/A/leased.txt", 1), get("/B/fine.txt", 2)], &mut sink)?;
    assert_eq!(
        sink.events,
        ["begin 0 1", "end 0 true", "failed 1 PermissionDenied"]
    );

    // A revoked grant admits no entry.
    fixture.revoke_access()?;
    let outcomes = backend.put_batch(&[put("/A/revoked.txt", 1)], &mut &b"r"[..])?;
    assert!(describe(&outcomes)[0].starts_with("failed"));
    assert!(!fixture.root_a.join("revoked.txt").exists());
    Ok(())
}

#[test]
fn transfer_engine_task_share_put_batch_abort_leaves_nothing() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &fixture.backend;
    let second = 400 * 1024;
    let entries = [put("/A/first.txt", 4), put("/A/second.bin", second as u64)];
    // The first 256 KiB frame reaches the host (both stages exist), then the
    // source fails in the middle of the second file.
    let mut source = FailingSource {
        data: vec![7u8; 4 + second],
        position: 0,
        fail_at: 4 + 300 * 1024,
    };
    let outcomes = backend.put_batch(&entries, &mut source)?;
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, BatchPutOutcome::Failed(_))),
        "{:?}",
        describe(&outcomes)
    );
    assert_eq!(stages_after_cleanup(&fixture.root_a)?, Vec::<String>::new());
    assert!(!fixture.root_a.join("first.txt").exists());
    assert!(!fixture.root_a.join("second.bin").exists());

    // The connection stays usable for the next batch.
    let outcomes = backend.put_batch(&[put("/A/after.txt", 2)], &mut &b"ok"[..])?;
    assert_eq!(describe(&outcomes), ["/A/after.txt"]);
    Ok(())
}

#[test]
fn transfer_engine_task_share_batch_status_resolves_a_lost_reply() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let peer = &fixture.peer;
    assert!(fixture.backend.batch_limits("/A").is_some());
    let endpoint = peer.current_endpoint()?;
    let commit = Ctrl::Fs {
        req: FsRequest::WriteDone,
        lease: None,
    };
    let header = |nonce: &str, path: &str, size: u64| Ctrl::Fs {
        req: FsRequest::PutBatch {
            nonce: nonce.to_string(),
            entries: vec![FsBatchPut {
                path: path.to_string(),
                size,
            }],
        },
        lease: None,
    };

    // The client "loses" the reply: it resets instead of confirming it.
    let lost = "00112233445566778899aabbccddeeff";
    let mut opened = peer.node.open_stream(&endpoint, &peer.identity)?;
    let replies = peer.node.block_on(async {
        send_ctrl(&mut opened.send, &header(lost, "/A/lost.txt", 4)).await?;
        send_tagged(&mut opened.send, TAG_DATA, b"lost").await?;
        send_ctrl(&mut opened.send, &commit).await?;
        let ready = recv_resp_wire(&mut opened.recv).await?;
        let done = recv_resp_wire(&mut opened.recv).await?;
        Ok::<_, io::Error>((ready, done))
    })?;
    let (ready, done) = replies;
    assert!(matches!(ready, FsResponse::Ready), "{ready:?}");
    let outcomes = match done {
        FsResponse::Batch {
            status: FsBatchStatus::Done { outcomes },
        } => outcomes,
        other => panic!("a committed batch expected: {other:?}"),
    };
    io_deadline::abort(&mut opened.send, &mut opened.recv);
    drop(opened);
    let status = peer.request(FsRequest::PutBatchStatus {
        nonce: lost.to_string(),
    })?;
    assert!(
        matches!(
            &status,
            FsResponse::Batch {
                status: FsBatchStatus::Done { outcomes: known },
            } if *known == outcomes
        ),
        "{status:?}"
    );
    assert_eq!(fs::read(fixture.root_a.join("lost.txt"))?, b"lost");

    // An unknown batch stays open; nothing is invented.
    let unknown = peer
        .request(FsRequest::PutBatchStatus {
            nonce: "ffffffffffffffff".into(),
        })
        .unwrap_err();
    assert_eq!(unknown.kind(), io::ErrorKind::NotFound);

    // A confirmed reply leaves no record behind.
    let confirmed = "0123456789abcdef0123456789abcdef";
    let mut opened = peer.node.open_stream(&endpoint, &peer.identity)?;
    peer.node.block_on(async {
        send_ctrl(&mut opened.send, &header(confirmed, "/A/kept.txt", 2)).await?;
        send_tagged(&mut opened.send, TAG_DATA, b"ok").await?;
        send_ctrl(&mut opened.send, &commit).await?;
        recv_resp_wire(&mut opened.recv).await?;
        recv_resp_wire(&mut opened.recv).await?;
        Ok::<_, io::Error>(())
    })?;
    let _ = opened.send.finish();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match peer.request(FsRequest::PutBatchStatus {
            nonce: confirmed.to_string(),
        }) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            other if Instant::now() >= deadline => {
                panic!("a delivered outcome must be dropped: {other:?}")
            }
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    assert_eq!(fs::read(fixture.root_a.join("kept.txt"))?, b"ok");
    Ok(())
}
