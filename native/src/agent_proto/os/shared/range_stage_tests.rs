//! The stage table of ranged uploads: taken and finished ranges, the space
//! all open stages reserve together, and what goes when a connection ends.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::{random_stage_name, RangeStages};

const MIB: u64 = 1024 * 1024;
const STALL: Duration = Duration::from_secs(600);

fn table(limit: usize) -> Arc<RangeStages> {
    RangeStages::new(limit, STALL, None)
}

/// What a host does: reserve, create exclusively, set the length, commit.
fn open(table: &Arc<RangeStages>, dir: &Path, name: &str, size: u64) -> io::Result<PathBuf> {
    let pending = table.begin(dir, size)?;
    let path = dir.join(name);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.set_len(size)?;
    pending.commit(name.to_string(), path.clone(), file)?;
    Ok(path)
}

const NAME: &str = "big.bin.se-upload-0123456789abcdef";

fn fill(table: &Arc<RangeStages>, offset: u64, len: u64) -> io::Result<()> {
    let mut claim = table.claim(NAME, offset, len)?;
    claim.write(&vec![7u8; len as usize])?;
    claim.complete()
}

#[test]
fn speed_task_range_stage_rejects_overlap_with_taken_and_finished_ranges() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(8);
    open(&table, dir.path(), NAME, 4 * MIB).expect("open");

    let first = table.claim(NAME, 0, MIB).expect("first range");
    let overlap = table
        .claim(NAME, MIB - 1, 2)
        .err()
        .expect("overlaps a taken range");
    assert_eq!(overlap.kind(), io::ErrorKind::InvalidInput);
    table
        .claim(NAME, MIB, MIB)
        .expect("a neighbour touching the taken range is fine")
        .complete()
        .expect_err("nothing was written into it");

    drop(first);
    fill(&table, 0, MIB).expect("a dropped claim frees its range");
    let finished = table
        .claim(NAME, 0, 1)
        .err()
        .expect("overlaps a finished range");
    assert_eq!(finished.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn speed_task_range_stage_rejects_ranges_outside_the_stage() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(8);
    open(&table, dir.path(), NAME, 2 * MIB).expect("open");
    for (offset, len) in [
        (0, 0),
        (0, 2 * MIB + 1),
        (2 * MIB, 1),
        (MIB, MIB + 1),
        (u64::MAX, 2),
        (1, u64::MAX),
    ] {
        let error = table
            .claim(NAME, offset, len)
            .err()
            .unwrap_or_else(|| panic!("({offset}, {len}) is outside the stage"));
        assert_eq!(
            error.kind(),
            io::ErrorKind::InvalidInput,
            "({offset}, {len})"
        );
    }
    assert!(table.claim(NAME, MIB, MIB).is_ok(), "the last range fits");
}

#[test]
fn speed_task_range_stage_write_refuses_more_than_announced() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(8);
    open(&table, dir.path(), NAME, MIB).expect("open");
    let mut claim = table.claim(NAME, 0, 100).expect("claim");
    claim.write(&[1; 60]).expect("within");
    let error = claim.write(&[1; 41]).expect_err("one byte too many");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let error = claim.complete().expect_err("only 60 of 100 came");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn speed_task_range_stage_finish_requires_full_coverage() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(8);
    let path = open(&table, dir.path(), NAME, 3 * MIB).expect("open");
    fill(&table, 0, MIB).expect("first");
    fill(&table, 2 * MIB, MIB).expect("last");
    let gap = table.finish(NAME).expect_err("the middle is missing");
    assert_eq!(gap.kind(), io::ErrorKind::InvalidData);

    let running = table.claim(NAME, MIB, MIB).expect("middle");
    let busy = table.finish(NAME).expect_err("a range is still running");
    assert_eq!(busy.kind(), io::ErrorKind::InvalidInput);
    drop(running);

    fill(&table, MIB, MIB).expect("middle");
    table.finish(NAME).expect("everything is there");
    assert!(path.exists(), "a finished stage stays for publication");
    assert_eq!(std::fs::metadata(&path).expect("stage").len(), 3 * MIB);
    assert_eq!(
        table.claim(NAME, 0, 1).err().map(|e| e.kind()),
        Some(io::ErrorKind::NotFound),
        "a finished stage takes no more ranges"
    );
}

#[test]
fn speed_task_range_stage_data_lands_at_its_offsets() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(8);
    let path = open(&table, dir.path(), NAME, 12).expect("open");
    let mut tail = table.claim(NAME, 8, 4).expect("tail");
    let mut head = table.claim(NAME, 0, 8).expect("head");
    tail.write(b"WXYZ").expect("tail data");
    head.write(b"abcd").expect("head data");
    head.write(b"efgh").expect("head data");
    tail.complete().expect("tail");
    head.complete().expect("head");
    table.finish(NAME).expect("finish");
    assert_eq!(std::fs::read(path).expect("stage"), b"abcdefghWXYZ");
}

#[test]
fn speed_task_range_stage_closing_removes_only_unfinished_stages() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(8);
    let unfinished = open(&table, dir.path(), NAME, MIB).expect("open");
    let done_name = "done.bin.se-upload-fedcba9876543210";
    let finished = open(&table, dir.path(), done_name, 4).expect("open");
    let mut claim = table.claim(done_name, 0, 4).expect("claim");
    claim.write(b"data").expect("write");
    claim.complete().expect("complete");
    table.finish(done_name).expect("finish");

    assert_eq!(table.open_count(), 1);
    table.close();
    assert!(!unfinished.exists(), "the connection ended: stage removed");
    assert!(finished.exists(), "a finished stage is kept");
    let error = table
        .begin(dir.path(), 1)
        .err()
        .expect("a closed table opens nothing");
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
}

#[test]
fn speed_task_range_stage_discard_removes_the_file_and_forgets_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(8);
    let path = open(&table, dir.path(), NAME, MIB).expect("open");
    assert!(table.holds(NAME));
    assert!(table.discard(NAME));
    assert!(!path.exists());
    assert!(!table.holds(NAME));
    assert!(!table.discard(NAME), "nothing left to discard");
}

#[test]
fn speed_task_range_stage_table_is_limited() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = table(2);
    open(&table, dir.path(), "a.se-upload-0000000000000001", MIB).expect("one");
    open(&table, dir.path(), "b.se-upload-0000000000000002", MIB).expect("two");
    let error = table
        .begin(dir.path(), MIB)
        .err()
        .expect("a third stage is refused");
    assert_eq!(error.kind(), io::ErrorKind::ResourceBusy);
    assert!(table.discard("a.se-upload-0000000000000001"));
    open(&table, dir.path(), "c.se-upload-0000000000000003", MIB).expect("room again");
}

#[test]
fn speed_task_range_stage_sums_the_space_of_all_open_stages() {
    let _serial = super::super::range_space::space_test_lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let available = super::local_platform::available_space(dir.path()).expect("free space");
    assert!(available > 64 * MIB, "the test needs some free space");
    let size = available / 10 * 6;
    let first = table(8);
    let second = table(8);
    open(&first, dir.path(), "a.se-upload-0000000000000001", size).expect("fits alone");
    let error = open(&second, dir.path(), "b.se-upload-0000000000000002", size)
        .expect_err("together the stages overbook the volume");
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    first.close();
    open(&second, dir.path(), "b.se-upload-0000000000000002", size).expect("space is back");
    let huge = second
        .begin(dir.path(), u64::MAX / 2)
        .err()
        .expect("more than the disk");
    assert_eq!(huge.kind(), io::ErrorKind::StorageFull);
}

#[test]
fn speed_task_range_stage_expires_without_progress() {
    let dir = tempfile::tempdir().expect("temp dir");
    let table = RangeStages::new(8, Duration::ZERO, None);
    let path = open(&table, dir.path(), NAME, MIB).expect("open");
    table.sweep();
    assert!(!path.exists(), "no progress within the stall time");
    assert_eq!(table.open_count(), 0);
}

#[test]
fn speed_task_range_stage_name_is_random_and_fits() {
    let first = random_stage_name("/data/video.mkv").expect("name");
    let second = random_stage_name("/data/video.mkv").expect("name");
    assert_ne!(first, second);
    let name = first.rsplit('/').next().expect("file name");
    assert!(crate::vfs::is_unique_stage(name), "{name}");
    assert!(first.starts_with("/data/video.mkv.se-upload-"));
    let long = format!("/data/{}", "ä".repeat(200));
    let fitted = random_stage_name(&long).expect("name");
    assert!(fitted.rsplit('/').next().expect("name").len() <= 255);
}
