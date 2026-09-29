//! Loopback coverage of the Share host's transfer limits (Block B1 review):
//! data frames of one chunk, stalled clients losing their slots, and stage
//! discards restricted to the engine's own upload stages.
use std::fs;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use super::transfer_engine_task_support::{stage_names, stages_after_cleanup};
use super::CopyPastePeerFixture;
use crate::share::framing::{recv_resp_wire, send_ctrl, send_tagged, TAG_DATA};
use crate::share::io_deadline;
use crate::share::wire::{Ctrl, FsBatchPut, FsRequest, FsResponse};

fn fs_request(req: FsRequest) -> Ctrl {
    Ctrl::Fs { req, lease: None }
}

#[test]
fn transfer_engine_task_share_oversized_frames_are_refused() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let peer = &fixture.peer;
    assert!(fixture.backend.batch_limits("/A").is_some());
    let endpoint = peer.current_endpoint()?;
    let oversized = vec![1u8; crate::share::fs::CHUNK + 1];

    // A batch frame larger than one chunk ends the batch: nothing is
    // published and no stage remains.
    let mut batch = peer.node.open_stream(&endpoint, &peer.identity)?;
    let header = fs_request(FsRequest::PutBatch {
        nonce: "00112233445566778899aabbccddeeff".into(),
        entries: vec![FsBatchPut {
            path: "/A/huge.bin".into(),
            size: oversized.len() as u64,
        }],
    });
    let ready = peer.node.block_on(async {
        send_ctrl(&mut batch.send, &header).await?;
        send_tagged(&mut batch.send, TAG_DATA, &oversized).await?;
        recv_resp_wire(&mut batch.recv).await
    })?;
    assert!(matches!(ready, FsResponse::Ready), "{ready:?}");
    let ended = peer.node.block_on(io_deadline::run(
        "test batch end",
        recv_resp_wire(&mut batch.recv),
    ));
    assert!(ended.is_err(), "the host must end the batch: {ended:?}");
    assert_eq!(stages_after_cleanup(&fixture.root_a)?, Vec::<String>::new());
    assert!(!fixture.root_a.join("huge.bin").exists());

    // A write frame larger than one chunk ends the write before any byte
    // of it reaches the target.
    let mut write = peer.node.open_stream(&endpoint, &peer.identity)?;
    let open = fs_request(FsRequest::WriteNew {
        path: "/A/big-frame.bin".into(),
    });
    let ready = peer.node.block_on(async {
        send_ctrl(&mut write.send, &open).await?;
        recv_resp_wire(&mut write.recv).await
    })?;
    assert!(matches!(ready, FsResponse::Ready), "{ready:?}");
    let _ = peer
        .node
        .block_on(send_tagged(&mut write.send, TAG_DATA, &oversized));
    let ended = peer.node.block_on(io_deadline::run(
        "test write end",
        recv_resp_wire(&mut write.recv),
    ));
    assert!(ended.is_err(), "the host must end the write: {ended:?}");
    assert_eq!(fs::metadata(fixture.root_a.join("big-frame.bin"))?.len(), 0);

    // Regular transfers on the same connection keep working.
    let regular = vec![2u8; 3 * crate::share::fs::CHUNK + 5];
    let mut writer = fixture.backend.open_write("/A/regular.bin")?;
    writer.write_all(&regular)?;
    writer.flush()?;
    drop(writer);
    assert_eq!(
        fs::metadata(fixture.root_a.join("regular.bin"))?.len(),
        3 * crate::share::fs::CHUNK as u64 + 5
    );
    Ok(())
}

#[test]
fn transfer_engine_task_share_stalled_client_frees_its_slot() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fixture.shorten_host_stall(Duration::from_secs(3));
    let principal = fixture.client_principal()?;
    let peer = &fixture.peer;
    // More than a stream window (16 MiB) plus the host's buffers, so the
    // host blocks on a client that reads nothing.
    let big = vec![5u8; 24 * 1024 * 1024];
    fs::write(fixture.root_a.join("big.bin"), &big)?;
    let endpoint = peer.current_endpoint()?;

    let mut reading = peer.node.open_stream(&endpoint, &peer.identity)?;
    let read = fs_request(FsRequest::Read {
        path: "/A/big.bin".into(),
    });
    let header = peer.node.block_on(async {
        send_ctrl(&mut reading.send, &read).await?;
        recv_resp_wire(&mut reading.recv).await
    })?;
    assert!(
        matches!(header, FsResponse::Data { size } if size == big.len() as u64),
        "{header:?}"
    );
    let mut writing = peer.node.open_stream(&endpoint, &peer.identity)?;
    let open = fs_request(FsRequest::WriteNew {
        path: "/A/stalled.bin".into(),
    });
    let ready = peer.node.block_on(async {
        send_ctrl(&mut writing.send, &open).await?;
        recv_resp_wire(&mut writing.recv).await
    })?;
    assert!(matches!(ready, FsResponse::Ready), "{ready:?}");
    assert_eq!(crate::share::server::transfers_in_use(&principal), 2);

    // Neither stream moves: the host gives both up after its stall bound.
    let deadline = Instant::now() + Duration::from_secs(30);
    while crate::share::server::transfers_in_use(&principal) > 0 {
        assert!(
            Instant::now() < deadline,
            "stalled transfers keep their slots"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(reading);
    drop(writing);
    assert_eq!(fs::metadata(fixture.root_a.join("stalled.bin"))?.len(), 0);
    Ok(())
}

#[test]
fn transfer_engine_task_share_discard_accepts_only_upload_stages() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &fixture.backend;
    assert!(fixture.backend.batch_limits("/A").is_some());
    let protected = [
        "user.txt",
        "x.txt.se-peer-0123456789abcdef",
        "x.txt.se-batch-0011223344556677-0",
        "x.txt.se-upload-0123456789ABCDEF",
        ".se-upload-0123456789abcdef",
    ];
    for name in protected {
        fs::write(fixture.root_a.join(name), b"keep")?;
        let refused = fixture
            .peer
            .request(FsRequest::DiscardStage {
                path: format!("/A/{name}"),
            })
            .unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{name}");
        assert_eq!(fs::read(fixture.root_a.join(name))?, b"keep", "{name}");
    }

    // An engine stage opened through the plain exclusive writer (as the
    // background service forwards it) is the client's own and can go.
    let stage = "/A/f.txt.se-upload-0123456789abcdef";
    let mut writer = backend.open_write_new(stage)?;
    writer.write_all(b"abc")?;
    writer.flush()?;
    drop(writer);
    backend.discard_copy_stage(stage)?;
    assert!(!fixture
        .root_a
        .join("f.txt.se-upload-0123456789abcdef")
        .exists());
    let again = backend.discard_copy_stage(stage).unwrap_err();
    assert_eq!(again.kind(), io::ErrorKind::Unsupported);

    // A stage of that shape this client did not create stays untouched.
    fs::write(
        fixture.root_a.join("g.txt.se-upload-00000000000000ff"),
        b"other",
    )?;
    let foreign = backend
        .discard_copy_stage("/A/g.txt.se-upload-00000000000000ff")
        .unwrap_err();
    assert_eq!(foreign.kind(), io::ErrorKind::Unsupported);
    assert!(fixture
        .root_a
        .join("g.txt.se-upload-00000000000000ff")
        .exists());
    assert_eq!(stage_names(&fixture.root_b)?, Vec::<String>::new());
    Ok(())
}
