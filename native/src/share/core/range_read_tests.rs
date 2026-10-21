//! Reads of one range on the Share (`ranges_v1`): the exact window, the end
//! of the file, the clamp behind it, hosts without the feature and the rights
//! a range read needs.
use std::fs;
use std::io::{self, Read};

use super::stage_ranges_tests::ctrl;
use super::transfer_engine_task_support::pattern;
use super::CopyPastePeerFixture;
use crate::share::framing::{recv_resp_wire, send_ctrl};
use crate::share::io_deadline;
use crate::share::wire::{FsRequest, FsResponse};
use crate::vfs::Backend;

const MIB: usize = 1024 * 1024;

fn read_all(mut reader: Box<dyn Read + Send>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[test]
fn speed_task_share_read_at_len_returns_the_exact_window() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let body = pattern(MIB, 251);
    fs::write(fixture.root_a.join("data.bin"), &body)?;
    let backend = &fixture.backend;
    assert!(backend.supports_range_reads("/A/data.bin"));
    let window = backend
        .open_read_range("/A/data.bin", None, 300_000, 50_000)?
        .expect("the host reads ranges");
    assert_eq!(read_all(window)?, &body[300_000..350_000]);
    // Ranges next to each other, as separate requests, make up the file.
    let first = read_all(
        backend
            .open_read_range("/A/data.bin", None, 0, 400_000)?
            .unwrap(),
    )?;
    let second = read_all(
        backend
            .open_read_range("/A/data.bin", None, 400_000, MIB as u64)?
            .unwrap(),
    )?;
    assert_eq!([first, second].concat(), body);
    assert!(read_all(backend.open_read_range("/A/data.bin", None, 5, 0)?.unwrap())?.is_empty());
    Ok(())
}

#[test]
fn speed_task_share_read_at_len_zero_reads_to_the_end() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let body = pattern(MIB, 241);
    fs::write(fixture.root_a.join("data.bin"), &body)?;
    let rest = fixture
        .backend
        .open_read_at("/A/data.bin", None, 900_000)?
        .expect("resume is supported");
    assert_eq!(read_all(rest)?, &body[900_000..]);
    let at_end = fixture
        .backend
        .open_read_at("/A/data.bin", None, MIB as u64)?
        .expect("resume is supported");
    assert!(read_all(at_end)?.is_empty());
    Ok(())
}

#[test]
fn speed_task_share_read_at_len_beyond_the_end_is_clamped() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let body = pattern(MIB, 239);
    fs::write(fixture.root_a.join("data.bin"), &body)?;
    let tail = fixture
        .backend
        .open_read_range("/A/data.bin", None, MIB as u64 - 1000, 10 * MIB as u64)?
        .expect("ranges");
    assert_eq!(read_all(tail)?, &body[MIB - 1000..]);
    let behind = fixture
        .backend
        .open_read_range("/A/data.bin", None, MIB as u64 + 1, 10)?;
    assert!(behind.is_err(), "a start behind the end is an error");
    // The announced length is what is left, not what was asked for.
    let request = FsRequest::ReadAt {
        path: "/A/data.bin".into(),
        id: None,
        offset: MIB as u64 - 7,
        len: u64::MAX,
        lz4: false,
    };
    let peer = &fixture.peer;
    let endpoint = peer.current_endpoint()?;
    let mut opened = peer.node.open_stream(&endpoint, &peer.identity)?;
    let header = peer.node.block_on(io_deadline::run("test read", async {
        send_ctrl(&mut opened.send, &ctrl(request)).await?;
        recv_resp_wire(&mut opened.recv).await
    }))?;
    assert!(matches!(header, FsResponse::Data { size: 7 }), "{header:?}");
    Ok(())
}

#[test]
fn speed_task_share_read_range_is_none_without_ranges_capability() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fixture.pose_as_legacy_host();
    fs::write(fixture.root_a.join("data.bin"), pattern(1000, 7))?;
    let backend = &fixture.backend;
    assert!(!backend.supports_range_reads("/A/data.bin"));
    assert!(backend
        .open_read_range("/A/data.bin", None, 10, 100)?
        .is_none());
    assert!(backend
        .open_segmented_stage("/A/new.bin.se-upload-0123456789abcdef", 100)?
        .is_none());
    Ok(())
}

#[test]
fn speed_task_share_read_at_len_needs_the_same_rights_as_read_at() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fs::write(fixture.root_a.join("data.bin"), pattern(5000, 11))?;
    let backend = &fixture.backend;
    assert!(backend
        .open_read_range("/A/data.bin", None, 0, 10)?
        .is_some());
    assert!(
        backend
            .open_read_range("/Unexported/data.bin", None, 0, 10)
            .is_err(),
        "outside the exports"
    );
    // A read-only relationship reads ranges like any read.
    fixture.set_client_may_write(false)?;
    assert!(backend
        .open_read_range("/A/data.bin", None, 0, 10)?
        .is_some());
    fixture.revoke_access()?;
    assert!(backend.open_read_at("/A/data.bin", None, 10).is_err());
    assert!(backend.open_read_range("/A/data.bin", None, 0, 10).is_err());
    Ok(())
}
