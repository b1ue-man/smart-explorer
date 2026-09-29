//! Loopback coverage of Share transfer v1 downloads and single files (Block
//! B1): batches down, a host before v1, admission, server copy, directories
//! and resume.
use std::fs;
use std::io::{self, Read, Write};

use super::transfer_engine_task_support::{get, pattern, put, RecordingSink, RefusingSink};
use super::CopyPastePeerFixture;
use crate::share::keepalive::TRANSFER_STREAMS_PER_CONNECTION;
use crate::share::wire::{FsRequest, FsResponse, FsTransferCapabilities};

#[test]
fn transfer_engine_task_share_get_batch_streams_items_in_order() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &fixture.backend;
    let big = pattern(700 * 1024, 253);
    fs::write(fixture.root_a.join("alpha.txt"), b"alpha")?;
    fs::write(fixture.root_a.join("changed.txt"), b"abc")?;
    fs::write(fixture.root_a.join("big.bin"), &big)?;
    fs::write(fixture.root_b.join("empty.bin"), b"")?;
    fs::create_dir(fixture.root_a.join("folder"))?;
    let items = [
        get("/A/alpha.txt", 5),
        get("/A/missing.txt", 1),
        get("/A/changed.txt", 7),
        get("/A/big.bin", big.len() as u64),
        get("/B/empty.bin", 0),
        get("/A/folder", 0),
    ];
    let mut sink = RecordingSink::default();
    backend.get_batch(&items, &mut sink)?;
    let big_begin = format!("begin 3 {}", big.len());
    assert_eq!(
        sink.events,
        [
            "begin 0 5",
            "end 0 true",
            "failed 1 NotFound",
            "failed 2 Other",
            big_begin.as_str(),
            "end 3 true",
            "begin 4 0",
            "end 4 true",
            "failed 5 Other",
        ]
    );
    assert_eq!(sink.bytes[0], b"alpha");
    assert_eq!(sink.bytes[3], big);
    assert!(sink.bytes[4].is_empty());

    // A sink that refuses ends the call; the connection serves the next one.
    let refused = backend
        .get_batch(&[get("/A/big.bin", big.len() as u64)], &mut RefusingSink)
        .unwrap_err();
    assert_eq!(refused.to_string(), "Ziel ist voll");
    let mut again = RecordingSink::default();
    backend.get_batch(&[get("/A/alpha.txt", 5)], &mut again)?;
    assert_eq!(again.events, ["begin 0 5", "end 0 true"]);
    Ok(())
}

#[test]
fn transfer_engine_task_share_legacy_host_keeps_the_single_path() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fixture.pose_as_legacy_host();
    let backend = &fixture.backend;
    assert!(backend.batch_limits("/A").is_none());
    assert_eq!(backend.transfer_ceiling("/A"), Some(32));
    let refused = backend
        .put_batch(&[put("/A/x.txt", 1)], &mut &b"x"[..])
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::Unsupported);
    let mut sink = RecordingSink::default();
    let refused = backend
        .get_batch(&[get("/A/x.txt", 1)], &mut sink)
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::Unsupported);
    assert!(sink.events.is_empty());

    // The single path: a sized stage, then stat plus no-replace rename.
    {
        let mut writer = backend.open_write_copy_stage_sized("/A/legacy.txt.se-copy-test", 6)?;
        writer.write_all(b"legacy")?;
        writer.flush()?;
    }
    backend.promote_copy_stage("/A/legacy.txt.se-copy-test", "/A/legacy.txt")?;
    assert_eq!(fs::read(fixture.root_a.join("legacy.txt"))?, b"legacy");
    backend.create_dir_new("/A/neu")?;
    assert_eq!(
        backend.create_dir_new("/A/neu").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    backend.create_dir("/A/neu")?;
    assert!(fixture.root_a.join("neu").is_dir());
    assert_eq!(
        backend
            .discard_copy_stage("/A/legacy.txt")
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
    assert!(backend.open_read_at("/A/legacy.txt", None, 2)?.is_none());
    let mut text = String::new();
    backend
        .open_read_id("/A/legacy.txt", Some("provider-id"))?
        .read_to_string(&mut text)?;
    assert_eq!(text, "legacy");
    Ok(())
}

#[test]
fn transfer_engine_task_share_single_path_saves_round_trips() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &fixture.backend;

    // Admission and flow identity follow the host's Capabilities.
    assert_eq!(
        backend.transfer_ceiling("/A"),
        Some(TRANSFER_STREAMS_PER_CONNECTION as usize)
    );
    assert_eq!(backend.flow_key("/A/x"), backend.flow_key("/B/y"));
    assert!(backend.flow_key("/A").starts_with("share:direct:"));
    assert!(backend.batch_limits("/").is_none());
    assert!(backend.batch_limits("/Verbindungen").is_none());
    let reply = fixture.peer.request(FsRequest::Capabilities {
        path: "/A".into(),
        acquire_lease: false,
        lease_request_id: None,
    })?;
    let capabilities = match reply {
        FsResponse::Capabilities { capabilities, .. } => capabilities,
        other => panic!("capabilities expected: {other:?}"),
    };
    assert_eq!(capabilities.transfer, FsTransferCapabilities::host());

    // One directory level per request, exclusive where asked.
    backend.create_dir("/B/copies")?;
    backend.create_dir("/B/copies")?;
    assert_eq!(
        backend.create_dir_new("/B/copies").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    backend.create_dir_new("/B/copies/fresh")?;
    assert!(fixture.root_b.join("copies").join("fresh").is_dir());

    // Server copy into a private stage, published without replacing.
    let payload = pattern(300 * 1024, 239);
    let size = payload.len() as u64;
    fs::write(fixture.root_b.join("source.bin"), &payload)?;
    let copies = fixture.root_b.join("copies");
    assert_eq!(
        backend.server_copy_to_stage("/B/source.bin", "/B/copies/source.bin.se-copy-a", size)?,
        Some(size)
    );
    backend.promote_copy_stage("/B/copies/source.bin.se-copy-a", "/B/copies/source.bin")?;
    assert_eq!(fs::read(copies.join("source.bin"))?, payload);
    assert_eq!(
        backend.server_copy_to_stage("/B/source.bin", "/B/copies/source.bin.se-copy-b", size)?,
        Some(size)
    );
    let taken = backend
        .promote_copy_stage("/B/copies/source.bin.se-copy-b", "/B/copies/source.bin")
        .unwrap_err();
    assert_eq!(taken.kind(), io::ErrorKind::AlreadyExists);

    // Only an own, unpublished stage is discarded.
    assert_eq!(
        backend
            .discard_copy_stage("/B/copies/source.bin")
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
    backend.discard_copy_stage("/B/copies/source.bin.se-copy-b")?;
    assert!(!copies.join("source.bin.se-copy-b").exists());
    assert_eq!(fs::read(copies.join("source.bin"))?, payload);
    assert_eq!(
        backend
            .discard_copy_stage("/B/copies/source.bin.se-copy-b")
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );

    // Resume in the middle of the file.
    let mut rest = Vec::new();
    backend
        .open_read_at("/B/copies/source.bin", None, 1000)?
        .expect("a transfer v1 host resumes")
        .read_to_end(&mut rest)?;
    assert_eq!(rest, payload[1000..]);
    assert!(backend
        .open_read_at("/B/copies/source.bin", None, size + 1)
        .is_err());
    Ok(())
}
