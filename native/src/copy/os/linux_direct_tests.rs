use super::super::linux_direct_plan::*;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const GIB: u64 = 1024 * MIB;

/// Bytes that differ from their neighbours at every scale, so a chunk that
/// lands at the wrong offset, or twice, changes the file.
fn content(len: u64) -> Vec<u8> {
    (0..len)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 10) ^ (i >> 20)) as u8)
        .collect()
}

struct Pair {
    _dir: tempfile::TempDir,
    reader: File,
    writer: File,
    stage: std::path::PathBuf,
    bytes: Vec<u8>,
}

/// A source of `len` bytes and a stage sparse-sized to it.
fn pair(len: u64) -> Pair {
    let dir = tempfile::tempdir().expect("temp dir");
    let bytes = content(len);
    let source = dir.path().join("source.bin");
    std::fs::write(&source, &bytes).expect("source");
    let stage = dir.path().join("stage.bin");
    let writer = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&stage)
        .expect("stage");
    writer.set_len(len).expect("sparse stage");
    Pair {
        reader: File::open(&source).expect("reader"),
        writer,
        stage,
        bytes,
        _dir: dir,
    }
}

/// The request of a normal buffered copy: reads and writes at the offsets
/// without O_DIRECT, so the cutting, the threads, the rest and the counting
/// are checked on every host.
fn plain_call(
    reader: &File,
    writer: &File,
    from: u64,
    to: u64,
    length: usize,
) -> io::Result<usize> {
    let mut buffer = vec![0u8; length.min(CHUNK)];
    let read = read_full(reader, &mut buffer, from)?;
    write_full(writer, &buffer[..read], to)?;
    Ok(read)
}

fn einval(_: &File, _: &File, _: u64, _: u64, _: usize) -> io::Result<usize> {
    Err(io::Error::from_raw_os_error(libc::EINVAL))
}

fn tagged_eopnotsupp(_: &File, _: &File, _: u64, _: u64, _: usize) -> io::Result<usize> {
    Err(tag(LocalFailure::Target(io::Error::from_raw_os_error(
        libc::EOPNOTSUPP,
    ))))
}

fn eio(_: &File, _: &File, _: u64, _: u64, _: usize) -> io::Result<usize> {
    Err(io::Error::from_raw_os_error(libc::EIO))
}

static CALLS: AtomicUsize = AtomicUsize::new(0);

/// The first request works, every later one is refused (tagged like the real
/// request does, so the range code does not read the refusal as its own
/// fallback signal): not a reason to start over (bytes are in the stage and
/// counted), the copy fails.
fn first_then_einval(
    reader: &File,
    writer: &File,
    from: u64,
    to: u64,
    length: usize,
) -> io::Result<usize> {
    if CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
        return plain_call(reader, writer, from, to, length);
    }
    Err(tag(LocalFailure::Target(io::Error::from_raw_os_error(
        libc::EINVAL,
    ))))
}

fn parts(pair: &Pair, size: u64, main: u64, call: RangeCall) -> Direct {
    Direct::from_parts(
        pair.reader.try_clone().expect("reader"),
        pair.writer.try_clone().expect("writer"),
        size,
        main,
        call,
    )
}

#[test]
fn speed_task_local_direct_threshold_is_a_quarter_of_the_available_memory() {
    let memory = Some(16 * GIB);
    assert!(wanted(4 * GIB, memory), "exactly a quarter");
    assert!(!wanted(4 * GIB - 1, memory), "one byte below");
    assert!(wanted(4 * GIB + 1, memory));
    assert!(
        !wanted(4 * GIB, Some(16 * GIB + 4)),
        "memory grew by 4 bytes"
    );
    // An unreadable /proc/meminfo never qualifies, and neither does nothing.
    assert!(!wanted(100 * GIB, None));
    assert!(!wanted(0, memory));
}

#[test]
fn speed_task_local_direct_main_part_and_rest_follow_the_block_unit() {
    assert_eq!(split_main(20 * MIB, 4096), (20 * MIB, 0), "a multiple");
    assert_eq!(split_main(20 * MIB + 1, 4096), (20 * MIB, 1));
    assert_eq!(split_main(20 * MIB + 4095, 4096), (20 * MIB, 4095));
    assert_eq!(split_main(20 * MIB + 4096, 4096), (20 * MIB + 4096, 0));
    assert_eq!(split_main(20 * MIB + 511, 512), (20 * MIB, 511));
    assert_eq!(split_main(4095, 4096), (0, 4095), "shorter than a block");
    for size in [1, 4097, 16 * MIB + 7, 4 * GIB + 4095] {
        let (main, rest) = split_main(size, 4096);
        assert_eq!(main % 4096, 0, "{size}");
        assert!(rest < 4096, "{size}");
        assert_eq!(main + rest, size);
    }
}

#[test]
fn speed_task_local_direct_unit_comes_from_the_reports_or_is_4096() {
    assert_eq!(unit_of([None, None]), Some(4096), "no report at all");
    assert_eq!(unit_of([Some((512, 512)), Some((512, 512))]), Some(512));
    assert_eq!(unit_of([Some((4, 512)), Some((512, 4096))]), Some(4096));
    assert_eq!(
        unit_of([Some((512, 512)), None]),
        Some(4096),
        "one is unknown"
    );
    assert_eq!(unit_of([Some((4096, 65536)), None]), Some(65536));
    // A file that takes no direct I/O reports 0; larger alignments than the
    // buffers have, or non-powers of two, are refused as well.
    assert_eq!(unit_of([Some((0, 0)), None]), None);
    assert_eq!(unit_of([Some((4, 0)), None]), None);
    assert_eq!(unit_of([None, Some((0, 512))]), None);
    assert_eq!(unit_of([Some((8192, 512)), None]), None);
    assert_eq!(unit_of([Some((4, 131_072)), None]), None);
    assert_eq!(unit_of([Some((4, 3000)), None]), None);
}

#[test]
fn speed_task_local_direct_needs_block_devices_on_both_sides() {
    assert!(block_backed((8 << 8) | 1), "sda1");
    assert!(block_backed((7 << 8) | 0), "loop0");
    assert!(block_backed(1 << 44), "a major above 4095");
    assert!(!block_backed(0), "tmpfs, NFS, FUSE");
    assert!(!block_backed(0x1_0000_00ff), "minor bits only");
}

#[test]
fn speed_task_local_direct_refusals_are_told_from_real_errors() {
    for errno in [
        libc::EINVAL,
        libc::EOPNOTSUPP,
        libc::ENOSYS,
        libc::EBADF,
        libc::ENOTTY,
        libc::EPERM,
    ] {
        assert!(
            is_unsupported(&io::Error::from_raw_os_error(errno)),
            "{errno}"
        );
    }
    for errno in [
        libc::EIO,
        libc::ENOSPC,
        libc::EDQUOT,
        libc::EROFS,
        libc::EACCES,
    ] {
        assert!(
            !is_unsupported(&io::Error::from_raw_os_error(errno)),
            "{errno}"
        );
    }
    assert!(!is_unsupported(&io::Error::other("no errno")));
    assert!(refused(&io::Error::from_raw_os_error(libc::EINVAL)));
    assert!(refused(&tag(LocalFailure::Source(
        io::Error::from_raw_os_error(libc::EOPNOTSUPP)
    ))));
    assert!(!refused(&tag(LocalFailure::Source(
        io::Error::from_raw_os_error(libc::EIO)
    ))));
    assert!(!refused(&io::Error::other("no errno")));
}

#[test]
fn speed_task_local_direct_buffers_are_aligned_at_every_size() {
    let mut storage = Vec::new();
    for len in [4096usize, 512, CHUNK, 8192, 4096, CHUNK, 1] {
        let buffer = aligned(&mut storage, len);
        assert_eq!(buffer.len(), len);
        assert_eq!(buffer.as_ptr() as usize % BUFFER_ALIGN, 0, "{len}");
        buffer.fill(7);
    }
    release_buffer();
}

#[test]
fn speed_task_local_direct_copies_a_standin_request_exactly_once_whatever_the_size() {
    // Sizes around the border: just above the engine's smallest parallel size,
    // a multiple of the block, one byte over it, nearly a block over it, odd.
    let never = AtomicBool::new(false);
    for size in [
        16 * MIB,
        16 * MIB + 1,
        16 * MIB + 4095,
        16 * MIB + 4096,
        20 * MIB + 1234,
        33 * MIB + 17,
    ] {
        let pair = pair(size);
        let (main, _) = split_main(size, 4096);
        let direct = parts(&pair, size, main, plain_call);
        let mut seen = Vec::new();
        let whole = direct
            .run(&pair.reader, &pair.writer, 4, &never, &mut |bytes| {
                seen.push(bytes)
            })
            .expect("copy");
        assert_eq!(whole, Whole::Done(size), "{size}");
        assert_eq!(seen.iter().sum::<u64>(), size, "{size}: every byte once");
        assert!(
            std::fs::read(&pair.stage).expect("stage") == pair.bytes,
            "{size}: the stage differs from the source"
        );
    }
}

#[test]
fn speed_task_local_direct_empty_and_short_files_have_no_direct_part() {
    // Nothing to copy directly: no handles are opened for it.
    let pair = pair(0);
    assert!(Direct::reopen(&pair.reader, &pair.writer, 0, plain_call).is_none());
    let pair = self::pair(4095);
    assert!(Direct::reopen(&pair.reader, &pair.writer, 4095, plain_call).is_none());
    // The older path takes them, empty file included, at any thread count.
    let never = AtomicBool::new(false);
    for size in [0u64, 1, 4095] {
        let pair = self::pair(size);
        let mut writer = pair.writer.try_clone().expect("writer");
        let mut seen = 0;
        let copied = super::super::copy_handles(
            &pair.reader,
            &mut writer,
            &never,
            &mut |bytes| seen += bytes,
            4,
        );
        assert_eq!(copied.expect("copy"), Some(size));
        assert_eq!(seen, size);
        assert!(std::fs::read(&pair.stage).expect("stage") == pair.bytes);
    }
}

#[test]
fn speed_task_local_direct_einval_on_the_first_request_falls_back_without_a_byte() {
    let never = AtomicBool::new(false);
    let size = 20 * MIB + 5;
    for call in [einval as RangeCall, tagged_eopnotsupp] {
        let pair = pair(size);
        let direct = parts(&pair, size, split_main(size, 4096).0, call);
        let mut seen = 0u64;
        let whole = direct
            .run(&pair.reader, &pair.writer, 4, &never, &mut |bytes| {
                seen += bytes
            })
            .expect("a refusal is no error");
        assert_eq!(whole, Whole::Unavailable);
        assert_eq!(seen, 0, "no progress before the fallback");
        assert!(
            std::fs::read(&pair.stage).expect("stage") == vec![0u8; size as usize],
            "the stage is untouched"
        );
        // The older path then copies the file, counting each byte once.
        let mut writer = pair.writer.try_clone().expect("writer");
        let copied = super::super::copy_handles(
            &pair.reader,
            &mut writer,
            &never,
            &mut |bytes| seen += bytes,
            1,
        );
        assert_eq!(copied.expect("fallback"), Some(size));
        assert_eq!(seen, size, "each byte once");
        assert!(std::fs::read(&pair.stage).expect("stage") == pair.bytes);
    }
}

#[test]
fn speed_task_local_direct_real_errors_are_not_a_fallback_and_keep_their_side() {
    let never = AtomicBool::new(false);
    let size = 20 * MIB;
    // A first request that fails for a real reason is the copy's error.
    let pair = pair(size);
    let direct = parts(&pair, size, size, eio);
    let error = direct
        .run(&pair.reader, &pair.writer, 4, &never, &mut |_| {})
        .expect_err("EIO is an error");
    assert_eq!(error.raw_os_error(), Some(libc::EIO));
    // A refusal after the first request is an error too: bytes are in the
    // stage and counted, starting over would count them twice.
    CALLS.store(0, Ordering::SeqCst);
    let direct = parts(&pair, size, size, first_then_einval);
    let mut seen = 0u64;
    let error = direct
        .run(&pair.reader, &pair.writer, 4, &never, &mut |bytes| {
            seen += bytes
        })
        .expect_err("a later EINVAL is an error");
    assert!(seen >= CHUNK as u64);
    drop(error);
    // The side: a read error belongs to the source, a write error to the
    // target, a source that ends early to the source.
    let dir = tempfile::tempdir().expect("dir");
    let folder = File::open(dir.path()).expect("folder handle");
    let writable = pair.writer.try_clone().expect("writer");
    let side = |error: io::Error| match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<LocalFailure>())
    {
        Some(LocalFailure::Source(inner)) => ("source", inner.kind()),
        Some(LocalFailure::Target(inner)) => ("target", inner.kind()),
        _ => ("untagged", error.kind()),
    };
    let error = direct_call(&folder, &writable, 0, 0, 4096).expect_err("reading a folder");
    assert_eq!(side(error).0, "source");
    let readonly = File::open(&pair.stage).expect("read-only stage");
    let error = direct_call(&pair.reader, &readonly, 0, 0, 4096).expect_err("read-only writer");
    assert_eq!(side(error).0, "target");
    let error = direct_call(&pair.reader, &writable, size - 100, 0, 4096).expect_err("short");
    assert_eq!(side(error), ("source", io::ErrorKind::UnexpectedEof));
    // The same request on a good pair moves the whole chunk.
    assert_eq!(
        direct_call(&pair.reader, &writable, 0, 0, 4096).expect("chunk"),
        4096
    );
}

#[test]
fn speed_task_local_direct_cancel_before_the_first_request_copies_nothing() {
    let pair = pair(20 * MIB);
    let direct = parts(&pair, 20 * MIB, 20 * MIB, plain_call);
    let canceled = AtomicBool::new(true);
    let mut seen = 0;
    let whole = direct
        .run(&pair.reader, &pair.writer, 4, &canceled, &mut |b| seen += b)
        .expect("cancel");
    assert_eq!(whole, Whole::Canceled);
    assert_eq!(seen, 0);
}

static STOPPING: AtomicBool = AtomicBool::new(false);

/// Copies, and raises the cancel flag with the second request.
fn cancel_after_first(
    reader: &File,
    writer: &File,
    from: u64,
    to: u64,
    length: usize,
) -> io::Result<usize> {
    if from > 0 {
        STOPPING.store(true, Ordering::SeqCst);
    }
    plain_call(reader, writer, from, to, length)
}

#[test]
fn speed_task_local_direct_cancel_in_the_ranges_ends_without_the_rest() {
    let size = 64 * MIB + 9;
    let pair = pair(size);
    let direct = parts(&pair, size, split_main(size, 4096).0, cancel_after_first);
    let whole = direct
        .run(&pair.reader, &pair.writer, 4, &STOPPING, &mut |_| {})
        .expect("cancel");
    assert_eq!(whole, Whole::Canceled);
}

#[test]
fn speed_task_local_direct_real_o_direct_copy_is_identical_or_falls_back_and_says_so() {
    // The real system: the temp folder may sit on tmpfs, overlay or a file
    // system that refuses O_DIRECT. Then the test checks the fallback and says
    // so; elsewhere it checks the bytes of a copy that really went direct.
    let never = AtomicBool::new(false);
    let sizes = [16 * MIB + 4096, 18 * MIB + 17, 20 * MIB];
    let mut direct_runs = 0;
    for size in sizes {
        let pair = pair(size);
        let Some(direct) = Direct::reopen(&pair.reader, &pair.writer, size, direct_call) else {
            eprintln!("note: no O_DIRECT handles here (no block device, /proc or flag): fallback");
            let mut writer = pair.writer.try_clone().expect("writer");
            let mut seen = 0;
            let copied = super::super::copy_handles(
                &pair.reader,
                &mut writer,
                &never,
                &mut |bytes| seen += bytes,
                4,
            );
            assert_eq!(copied.expect("fallback copy"), Some(size));
            assert_eq!(seen, size);
            assert!(std::fs::read(&pair.stage).expect("stage") == pair.bytes);
            continue;
        };
        let mut seen = 0u64;
        let whole = direct
            .run(&pair.reader, &pair.writer, 4, &never, &mut |bytes| {
                seen += bytes
            })
            .expect("copy");
        match whole {
            Whole::Done(copied) => {
                direct_runs += 1;
                assert_eq!(copied, size);
                assert_eq!(seen, size, "each byte once");
            }
            Whole::Unavailable => {
                eprintln!("note: the file system refused the first O_DIRECT request: fallback");
                assert_eq!(seen, 0);
                let mut writer = pair.writer.try_clone().expect("writer");
                let copied = super::super::copy_handles(
                    &pair.reader,
                    &mut writer,
                    &never,
                    &mut |bytes| seen += bytes,
                    1,
                );
                assert_eq!(copied.expect("fallback copy"), Some(size));
                assert_eq!(seen, size);
            }
            Whole::Canceled => unreachable!("nobody canceled"),
        }
        assert!(
            std::fs::read(&pair.stage).expect("stage") == pair.bytes,
            "{size}"
        );
    }
    eprintln!("note: {direct_runs} of {} copies went direct", sizes.len());
}

#[test]
fn speed_task_local_direct_reopen_names_the_same_file_and_never_creates_or_truncates() {
    // The handle opened through /proc/self/fd is the same inode, and opening
    // it again neither truncates the stage nor creates anything.
    let pair = pair(8192);
    pair.writer.write_all_at(b"keep", 0).expect("content");
    let before = std::fs::symlink_metadata(&pair.stage).expect("stage").len();
    if let Some(again) = open_same(&pair.writer, true) {
        assert_eq!(
            super::super::file_identity(&again).expect("identity"),
            super::super::file_identity(&pair.writer).expect("identity")
        );
        let mut head = [0u8; 4];
        pair.writer.read_at(&mut head, 0).expect("read");
        assert_eq!(&head, b"keep", "content kept");
    } else {
        eprintln!("note: O_DIRECT through /proc/self/fd is not possible here");
    }
    let after = std::fs::symlink_metadata(&pair.stage).expect("stage").len();
    assert_eq!(before, after, "not truncated");
    let names: Vec<_> = std::fs::read_dir(pair.stage.parent().expect("folder"))
        .expect("folder")
        .collect();
    assert_eq!(names.len(), 2, "no file was created");
}

#[test]
fn speed_task_local_direct_pairs_that_clone_stay_on_the_kernel_copy() {
    // Whether the temp folder's file system clones is up to the host; a pair
    // that clones is never copied directly.
    let pair = pair(20 * MIB);
    let clones = clones(&pair.reader, &pair.writer);
    let direct = Direct::reopen(&pair.reader, &pair.writer, 20 * MIB, plain_call);
    if clones {
        eprintln!("note: this file system clones extents: no direct copy");
        assert!(direct.is_none());
    }
}
