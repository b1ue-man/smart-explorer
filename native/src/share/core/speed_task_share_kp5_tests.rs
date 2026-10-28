//! Loopback coverage of KP5a over two real Iroh nodes: a revoked grant or
//! a withdrawn write right stops the running transfer at its next block and
//! the next request (Z-H1); a single file goes as a packet of one and is
//! published once (Z-P4); transfers use the extra connections of a host
//! that offers them, each with its full handshake (Z-P6).
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use super::transfer_engine_task_support::{pattern, put};
use super::CopyPastePeerFixture;
use crate::vfs::BatchPutOutcome;

/// Larger than the stream window (16 MiB) plus the host's read-ahead, so
/// the host is still reading when the right goes.
const LARGE: usize = 48 * 1024 * 1024;

#[test]
fn speed_task_share_revoked_grant_stops_the_running_read() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fs::write(fixture.root_a.join("large.bin"), pattern(LARGE, 251))?;
    let mut reader = fixture.backend.open_read("/A/large.bin")?;
    let mut block = vec![0u8; 256 * 1024];
    let first = reader.read(&mut block)?;
    assert!(first > 0, "the transfer runs");
    fixture.revoke_access()?;
    let mut rest = Vec::new();
    let finished = reader.read_to_end(&mut rest);
    assert!(
        finished.is_err() || first + rest.len() < LARGE,
        "a revoked grant must not deliver the whole file"
    );
    assert!(
        fixture.backend.stat("/A/large.bin").is_err(),
        "the next request is refused"
    );
    Ok(())
}

#[test]
fn speed_task_share_withdrawn_write_right_stops_the_running_upload() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let mut writer = fixture.backend.open_write_new("/B/upload.bin")?;
    let block = pattern(256 * 1024, 241);
    writer.write_all(&block)?;
    fixture.set_client_may_write(false)?;
    let mut refused = false;
    for _ in 0..(LARGE / block.len()) {
        if writer.write_all(&block).is_err() {
            refused = true;
            break;
        }
    }
    refused |= writer.flush().is_err();
    drop(writer);
    assert!(refused, "the host refuses blocks after the right went");
    let kept = fs::metadata(fixture.root_b.join("upload.bin")).map_or(0, |meta| meta.len());
    assert!(kept < LARGE as u64, "the host took {kept} bytes");
    fixture.set_client_may_write(true)?;
    // The right is back: the memo of the session follows the grant.
    let mut again = fixture.backend.open_write_new("/B/again.bin")?;
    again.write_all(b"again")?;
    again.flush()?;
    drop(again);
    assert_eq!(fs::read(fixture.root_b.join("again.bin"))?, b"again");
    Ok(())
}

#[test]
fn speed_task_share_single_file_goes_as_a_packet_of_one() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    assert!(fixture.peer.single_put_offered());
    fs::write(fixture.root_b.join("taken.txt"), b"older")?;
    let outcomes = fixture
        .backend
        .put_batch(&[put("/B/taken.txt", 5)], &mut &b"newer"[..])?;
    let published = match outcomes.as_slice() {
        [BatchPutOutcome::Published(path)] => path.clone(),
        other => return Err(io::Error::other(format!("{other:?}"))),
    };
    // Never replaced: the host numbers the new file.
    assert_ne!(published, "/B/taken.txt");
    assert_eq!(fs::read(fixture.root_b.join("taken.txt"))?, b"older");
    let name = published.trim_start_matches("/B/");
    assert_eq!(fs::read(fixture.root_b.join(name))?, b"newer");
    Ok(())
}

#[test]
fn speed_task_share_bulk_connections_carry_transfers() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let content = pattern(1024 * 1024, 239);
    fs::write(fixture.root_a.join("bulk.bin"), &content)?;
    // The first connection exists and has the host's capabilities.
    fixture.backend.stat("/A/bulk.bin")?;
    assert!(fixture.peer.bulk_offered());
    let endpoint = fixture.peer.current_endpoint()?;
    fixture.peer.node.want_bulk_connections(&endpoint, 2);
    let mut connections = HashSet::new();
    for _ in 0..6 {
        let deadline = Instant::now() + Duration::from_secs(20);
        let opened = fixture.peer.transfer_stream(&endpoint, deadline)?;
        connections.insert(opened.generation);
    }
    // Each extra connection passed hello, proof and admission, or the
    // stream would have fallen back to the first one.
    assert_eq!(connections.len(), 3, "{connections:?}");
    for _ in 0..6 {
        let mut bytes = Vec::new();
        fixture
            .backend
            .open_read("/A/bulk.bin")?
            .read_to_end(&mut bytes)?;
        assert_eq!(bytes, content);
    }
    // A revoked grant fails on every connection alike.
    fixture.revoke_access()?;
    for _ in 0..3 {
        assert!(fixture.backend.open_read("/A/bulk.bin").is_err());
    }
    fixture.peer.node.want_bulk_connections(&endpoint, 0);
    Ok(())
}
