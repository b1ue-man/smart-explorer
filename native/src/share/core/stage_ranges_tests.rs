//! Loopback coverage of ranged transfers on the Share (`ranges_v1`, U53):
//! reads of one range, the exclusive stage of a large upload and its ranges,
//! and what the host enforces (rights, overlap, space, connection, limits).
use std::fs;
use std::io::{self, Read, Write};
use std::time::Duration;

use super::transfer_engine_task_support::{pattern, stages_after_cleanup};
use super::CopyPastePeerFixture;
use crate::share::framing::{decode_resp, recv_resp_wire, send_ctrl, send_tagged, TAG_DATA};
use crate::share::io_deadline;
use crate::share::wire::{
    Ctrl, FsRequest, FsResponse, FsSegmentedFinish, FsSegmentedOpen, FsStageWrite,
};

const MIB: usize = 1024 * 1024;

pub(super) fn ctrl(req: FsRequest) -> Ctrl {
    Ctrl::Fs { req, lease: None }
}

/// One request on a stream of its own, answered once.
pub(super) fn call(fixture: &CopyPastePeerFixture, req: FsRequest) -> io::Result<FsResponse> {
    let peer = &fixture.peer;
    let endpoint = peer.current_endpoint()?;
    let mut opened = peer.node.open_stream(&endpoint, &peer.identity)?;
    let reply = peer.node.block_on(io_deadline::run("test request", async {
        send_ctrl(&mut opened.send, &ctrl(req)).await?;
        recv_resp_wire(&mut opened.recv).await
    }))?;
    decode_resp(reply)
}

pub(super) fn open_stage(
    fixture: &CopyPastePeerFixture,
    path: &str,
    size: u64,
) -> io::Result<String> {
    let request = FsRequest::SegmentedStageOpen(FsSegmentedOpen {
        path: path.to_string(),
        size,
    });
    match call(fixture, request)? {
        FsResponse::SegmentedStageReady { stage } => Ok(stage),
        other => Err(io::Error::other(format!("unexpected reply {other:?}"))),
    }
}

pub(super) fn write_request(stage: &str, offset: u64, len: u64) -> FsRequest {
    FsRequest::StageWriteAt(FsStageWrite {
        stage: stage.to_string(),
        offset,
        len,
        lz4: false,
    })
}

/// A range with the given data frames; the stream ends after the last one.
fn write_range(
    fixture: &CopyPastePeerFixture,
    stage: &str,
    (offset, len): (u64, u64),
    parts: &[&[u8]],
) -> io::Result<FsResponse> {
    let peer = &fixture.peer;
    let endpoint = peer.current_endpoint()?;
    let mut opened = peer.node.open_stream(&endpoint, &peer.identity)?;
    let request = ctrl(write_request(stage, offset, len));
    let ready = peer
        .node
        .block_on(io_deadline::run("test range open", async {
            send_ctrl(&mut opened.send, &request).await?;
            recv_resp_wire(&mut opened.recv).await
        }))?;
    match decode_resp(ready)? {
        FsResponse::Ready => {}
        other => return Err(io::Error::other(format!("unexpected reply {other:?}"))),
    }
    peer.node
        .block_on(io_deadline::run("test range data", async {
            for part in parts {
                send_tagged(&mut opened.send, TAG_DATA, part).await?;
            }
            opened.send.finish().map_err(io::Error::other)?;
            recv_resp_wire(&mut opened.recv).await
        }))
        .and_then(decode_resp)
}

fn finish(fixture: &CopyPastePeerFixture, stage: &str) -> io::Result<FsResponse> {
    call(
        fixture,
        FsRequest::SegmentedStageFinish(FsSegmentedFinish {
            stage: stage.to_string(),
        }),
    )
}

/// The stage as a file below the export `A`.
pub(super) fn on_disk(fixture: &CopyPastePeerFixture, stage: &str) -> std::path::PathBuf {
    fixture.root_a.join(stage.trim_start_matches("/A/"))
}

#[test]
fn speed_task_share_stage_open_creates_an_exclusive_sparse_stage() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let first = open_stage(&fixture, "/A/big.bin", 5 * MIB as u64 + 3)?;
    let second = open_stage(&fixture, "/A/big.bin", 10)?;
    assert_ne!(first, second, "the host picks a new random name each time");
    for stage in [&first, &second] {
        let name = stage
            .strip_prefix("/A/big.bin.se-upload-")
            .expect("host name");
        assert_eq!(name.len(), 16, "{stage}");
        assert!(name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    }
    assert_eq!(
        fs::metadata(on_disk(&fixture, &first))?.len(),
        5 * MIB as u64 + 3
    );
    assert!(!fixture.root_a.join("big.bin").exists());
    // The client names only the file: a stage name it made up is just a file name.
    let invented = open_stage(&fixture, "/A/x.bin.se-upload-0123456789abcdef", 1)?;
    assert_ne!(invented, "/A/x.bin.se-upload-0123456789abcdef");
    // Outside the exports, or without a file name, nothing is made.
    assert!(open_stage(&fixture, "/Unexported/big.bin", 1).is_err());
    assert!(open_stage(&fixture, "/A", 1).is_err());
    assert!(open_stage(&fixture, "/A/../escape.bin", 1).is_err());
    Ok(())
}

#[test]
fn speed_task_share_stage_write_at_rejects_overlap() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/o.bin", 4 * MIB as u64)?;
    let data = pattern(MIB, 253);
    write_range(&fixture, &stage, (0, MIB as u64), &[&data])?;
    let overlap = write_range(&fixture, &stage, (MIB as u64 / 2, MIB as u64), &[&data]);
    assert!(overlap.is_err(), "overlaps a finished range");

    // A range that is accepted but still running blocks an overlapping one.
    let peer = &fixture.peer;
    let endpoint = peer.current_endpoint()?;
    let mut running = peer.node.open_stream(&endpoint, &peer.identity)?;
    let request = ctrl(write_request(&stage, MIB as u64, MIB as u64));
    let ready = peer
        .node
        .block_on(io_deadline::run("test range open", async {
            send_ctrl(&mut running.send, &request).await?;
            recv_resp_wire(&mut running.recv).await
        }))?;
    assert!(matches!(ready, FsResponse::Ready), "{ready:?}");
    let blocked = call(&fixture, write_request(&stage, 2 * MIB as u64 - 1, 10));
    assert!(blocked.is_err(), "overlaps a range that was accepted");
    let next = write_range(&fixture, &stage, (2 * MIB as u64, 1000), &[&data[..1000]]);
    assert!(next.is_ok(), "a neighbour is fine");
    peer.node
        .block_on(io_deadline::run("test range data", async {
            send_tagged(&mut running.send, TAG_DATA, &data).await?;
            running.send.finish().map_err(io::Error::other)?;
            recv_resp_wire(&mut running.recv).await
        }))?;
    let on_disk = fs::read(on_disk(&fixture, &stage))?;
    assert_eq!(&on_disk[..MIB], &data[..]);
    assert_eq!(&on_disk[MIB..2 * MIB], &data[..]);
    Ok(())
}

#[test]
fn speed_task_share_stage_write_at_rejects_ranges_outside_the_stage() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/r.bin", 2 * MIB as u64)?;
    for (offset, len) in [
        (0, 0),
        (0, 2 * MIB as u64 + 1),
        (2 * MIB as u64, 1),
        (u64::MAX, 2),
        (1, u64::MAX),
    ] {
        let refused = call(&fixture, write_request(&stage, offset, len));
        assert!(refused.is_err(), "({offset}, {len})");
    }
    assert!(matches!(
        call(&fixture, write_request(&stage, MIB as u64, MIB as u64))?,
        FsResponse::Ready
    ));
    Ok(())
}

#[test]
fn speed_task_share_stage_write_at_rejects_short_or_long_data() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/s.bin", MIB as u64)?;
    assert!(write_range(&fixture, &stage, (0, 100), &[&[1; 99]]).is_err());
    assert!(write_range(&fixture, &stage, (0, 100), &[&[1; 101]]).is_err());
    assert!(write_range(&fixture, &stage, (0, 100), &[&[1; 60], &[1; 41]]).is_err());
    assert!(write_range(&fixture, &stage, (0, 100), &[]).is_err());
    // None of that counted: the range is free and takes exactly 100 bytes.
    write_range(&fixture, &stage, (0, 100), &[&[9; 100]])?;
    assert_eq!(
        &fs::read(on_disk(&fixture, &stage))?[..100],
        &[9u8; 100][..]
    );
    Ok(())
}

#[test]
fn speed_task_share_stage_finish_requires_full_coverage() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/f.bin", 3 * MIB as u64)?;
    let data = pattern(MIB, 247);
    write_range(&fixture, &stage, (0, MIB as u64), &[&data])?;
    write_range(&fixture, &stage, (2 * MIB as u64, MIB as u64), &[&data])?;
    assert!(finish(&fixture, &stage).is_err(), "the middle is missing");
    write_range(&fixture, &stage, (MIB as u64, MIB as u64), &[&data])?;
    assert!(matches!(finish(&fixture, &stage)?, FsResponse::Ok));
    assert!(on_disk(&fixture, &stage).exists(), "kept for publication");
    assert!(
        finish(&fixture, &stage).is_err(),
        "a finished stage is closed"
    );
    Ok(())
}

#[test]
fn speed_task_share_stage_is_removed_when_the_connection_ends() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/c.bin", MIB as u64)?;
    let done = open_stage(&fixture, "/A/d.bin", 4)?;
    write_range(&fixture, &done, (0, 4), &[b"data"])?;
    finish(&fixture, &done)?;
    assert!(on_disk(&fixture, &stage).exists());
    fixture.close_connection()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while on_disk(&fixture, &stage).exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!on_disk(&fixture, &stage).exists(), "the host removed it");
    assert!(on_disk(&fixture, &done).exists(), "a finished stage stays");
    Ok(())
}

#[test]
fn speed_task_share_stage_belongs_to_the_opening_connection() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/b.bin", MIB as u64)?;
    fixture.close_connection()?;
    // The next request runs on a new connection, which never opened it.
    let refused = write_range(&fixture, &stage, (0, 10), &[&[1; 10]]);
    assert!(refused.is_err());
    assert!(finish(&fixture, &stage).is_err());
    Ok(())
}

#[test]
fn speed_task_share_stage_write_is_denied_in_a_read_only_room() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/w.bin", MIB as u64)?;
    fixture.set_client_may_write(false)?;
    let open = open_stage(&fixture, "/A/ro.bin", 10);
    assert_eq!(
        open.expect_err("read-only").kind(),
        io::ErrorKind::ReadOnlyFilesystem
    );
    assert!(write_range(&fixture, &stage, (0, 10), &[&[1; 10]]).is_err());
    assert!(
        !on_disk(&fixture, &stage).exists(),
        "the stage goes with the right to write it"
    );
    fixture.set_client_may_write(true)?;
    assert!(
        finish(&fixture, &stage).is_err(),
        "and it is not there any more"
    );
    assert!(stages_after_cleanup(&fixture.root_a)?.is_empty());
    Ok(())
}

#[test]
fn speed_task_share_stage_write_is_denied_after_the_grant_is_revoked() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/g.bin", MIB as u64)?;
    fixture.revoke_access()?;
    assert!(write_range(&fixture, &stage, (0, 10), &[&[1; 10]]).is_err());
    assert!(finish(&fixture, &stage).is_err());
    Ok(())
}

#[test]
fn speed_task_share_stage_open_needs_free_space() -> io::Result<()> {
    let _serial = crate::agent_proto::space_test_lock();
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let refused = open_stage(&fixture, "/A/huge.bin", u64::MAX / 4);
    assert_eq!(
        refused.expect_err("more than the disk").kind(),
        io::ErrorKind::StorageFull
    );
    assert!(stages_after_cleanup(&fixture.root_a)?.is_empty());

    // The space that open stages still have to fill is counted together.
    let available = crate::agent_proto::available_space(&fixture.root_a)?;
    let size = available / 10 * 6;
    let first = open_stage(&fixture, "/A/one.bin", size)?;
    let second = open_stage(&fixture, "/A/two.bin", size);
    assert_eq!(
        second.expect_err("together too much").kind(),
        io::ErrorKind::StorageFull
    );
    call(&fixture, FsRequest::DiscardStage { path: first })?;
    open_stage(&fixture, "/A/two.bin", size)?;
    Ok(())
}

#[test]
fn speed_task_share_stage_table_is_limited_by_admission() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let limit = crate::share::keepalive::TRANSFER_STREAMS_PER_CONNECTION as usize;
    let mut stages = Vec::new();
    for index in 0..limit {
        stages.push(open_stage(&fixture, &format!("/A/many{index}.bin"), 1)?);
    }
    let refused = open_stage(&fixture, "/A/one-too-many.bin", 1).expect_err("Busy");
    assert!(crate::vfs::congestion_of(&refused).is_some(), "{refused:?}");
    call(
        &fixture,
        FsRequest::DiscardStage {
            path: stages.remove(0),
        },
    )?;
    open_stage(&fixture, "/A/room-again.bin", 1)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn speed_task_share_stage_write_goes_through_the_held_handle() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let stage = open_stage(&fixture, "/A/h.bin", 1000)?;
    let path = on_disk(&fixture, &stage);
    // Someone replaces the stage by path after it was made.
    fs::rename(&path, fixture.root_a.join("moved.bin"))?;
    fs::write(&path, vec![0u8; 1000])?;
    write_range(&fixture, &stage, (0, 1000), &[&[5; 1000]])?;
    assert_eq!(
        fs::read(&path)?,
        vec![0u8; 1000],
        "the stranger is untouched"
    );
    assert_eq!(fs::read(fixture.root_a.join("moved.bin"))?, vec![5u8; 1000]);
    Ok(())
}

#[test]
fn speed_task_share_segments_end_to_end_between_two_peers() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &fixture.backend;
    let size = 40 * MIB + 12_345;
    let body = pattern(size, 251);
    let hint = "/A/video.mkv.se-upload-0123456789abcdef";
    let stage = backend
        .open_segmented_stage(hint, size as u64)?
        .expect("the host offers ranged stages");
    let name = stage.stage_path().to_string();
    assert!(name.starts_with("/A/video.mkv.se-upload-"));
    assert_ne!(name, hint, "the host picked the name");

    let step = size / 6 + 1;
    std::thread::scope(|scope| {
        let (stage, body) = (&*stage, &body);
        for from in (0..size).step_by(step).rev() {
            let to = (from + step).min(size);
            scope.spawn(move || {
                let mut writer = stage
                    .segment(from as u64, (to - from) as u64)
                    .expect("range");
                writer.write_all(&body[from..to]).expect("data");
                writer.flush().expect("range complete");
            });
        }
    });
    stage.finish()?;
    assert!(!fixture.root_a.join("video.mkv").exists());
    // Published without replacing, as the engine does.
    backend.promote_staged_no_replace(&name, "/A/video.mkv")?;
    assert_eq!(fs::read(fixture.root_a.join("video.mkv"))?, body);
    assert!(stages_after_cleanup(&fixture.root_a)?.is_empty());
    let again = backend.open_segmented_stage(hint, 10)?.expect("stage");
    let second = again.stage_path().to_string();
    let mut range = again.segment(0, 10)?;
    range.write_all(b"not stored")?;
    range.flush()?;
    drop(range);
    again.finish()?;
    let taken = backend.promote_staged_no_replace(&second, "/A/video.mkv");
    assert!(taken.is_err(), "an existing file is never replaced");
    assert_eq!(fs::read(fixture.root_a.join("video.mkv"))?, body);
    Ok(())
}

#[test]
fn speed_task_share_old_peer_without_ranges_uses_one_stream() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fixture.pose_as_legacy_host();
    let backend = &fixture.backend;
    assert!(backend
        .open_segmented_stage("/A/old.bin.se-upload-0123456789abcdef", 50)?
        .is_none());
    let mut writer = backend.open_write_new("/A/old.bin")?;
    writer.write_all(&pattern(50, 9))?;
    writer.flush()?;
    drop(writer);
    assert_eq!(fs::read(fixture.root_a.join("old.bin"))?, pattern(50, 9));
    let mut read_back = Vec::new();
    backend
        .open_read("/A/old.bin")?
        .read_to_end(&mut read_back)?;
    assert_eq!(read_back, pattern(50, 9));
    Ok(())
}
