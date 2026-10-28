//! The G3 follow-up items against real servers (plan 2026-10-28, G3-07 and
//! G3-12; G3-08b needs a Drive account and runs against the loopback Drive
//! in `gdrive/core/absent_write_tests.rs` and, through the engine, in
//! `gdrive/core/absent_engine_task_tests.rs`).
//!
//! - SMB (`SE_TASK_SMB`): an engine copy stage that fits one WRITE commits
//!   as WRITE + CLOSE in one compound. Its exclusive CREATE still answers a
//!   taken name at the open, nothing is ever replaced, a stage dropped or
//!   short is removed, and its evidence comes from the CLOSE. Reads open
//!   with their first READ in the same compound; the bytes must be exact
//!   below, at and above one READ block.
//! - Nextcloud (`SE_TASK_NEXTCLOUD`): a 2 GiB upload goes as parallel chunks
//!   and must come back byte for byte.
//! - SFTP (`SE_TASK_SFTP`): server-side copies of a packet in one call, and a
//!   same-account copy through the engine as packets of server copies,
//!   byte-exact and never replacing.
use crate::speed::{bench_job, local_case, report, same_bytes, LARGE, TOP};
use crate::support::{job, local_folder, nextcloud, remote_folder, slash, smb};
use smart_explorer::transfer::Endpoint;
use smart_explorer::vfs::{self, Backend};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

fn write_new(backend: &dyn Backend, path: &str, content: &[u8]) {
    let mut writer = backend.open_write_new(path).expect("create file");
    writer.write_all(content).expect("write file");
    writer.flush().expect("commit file");
}

fn read_all(reader: &mut dyn Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read");
    bytes
}

fn read_path(backend: &dyn Backend, path: &str) -> Vec<u8> {
    read_all(&mut *backend.open_read(path).expect("open"))
}

fn kind<T>(result: std::io::Result<T>) -> Option<ErrorKind> {
    result.err().map(|error| error.kind())
}

fn content(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

#[test]
#[ignore = "needs the task suite's SMB server"]
fn speed_task_remote_smb_compound_stage_never_replaces() {
    let Some((backend, root)) = smb() else {
        return;
    };
    let folder = remote_folder(&*backend, &root, "g3-07");
    let taken = format!("{folder}/taken.bin");
    write_new(&*backend, &taken, b"theirs");

    // A taken name is refused at the open, as the engine's stage loop expects.
    let refused = backend.open_write_copy_stage_unsynced(&taken, 5);
    assert_eq!(kind(refused), Some(ErrorKind::AlreadyExists));
    assert_eq!(read_path(&*backend, &taken), b"theirs");

    // One compound commits the stage; its evidence comes with the CLOSE.
    let stage = format!("{folder}/neu.bin.se-upload-0123456789abcdef");
    let published = format!("{folder}/neu.bin");
    let mut writer = backend.open_write_copy_stage_unsynced(&stage, 5).unwrap();
    writer.write_all(b"hello").unwrap();
    writer.flush().unwrap();
    drop(writer);
    let evidence = vfs::take_stage_evidence(&*backend, &stage);
    let evidence = evidence.expect("the commit reported the stage");
    assert_eq!(evidence.stage.size, 5);
    assert_eq!(evidence.destination, Some(published.clone()));
    assert_eq!(evidence.existed, Some(false));

    // Publishing onto a taken name is refused; the stage stays and goes on.
    let onto_taken = vfs::promote_copy_stage_evidence(&*backend, &stage, &taken);
    assert_eq!(kind(onto_taken), Some(ErrorKind::AlreadyExists));
    assert_eq!(read_path(&*backend, &taken), b"theirs");
    let moved = vfs::promote_copy_stage_evidence(&*backend, &stage, &published);
    assert!(moved.is_ok(), "{moved:?}");
    assert_eq!(read_path(&*backend, &published), b"hello");

    // Dropped before the commit or short at it: nothing stays behind.
    let dropped = format!("{folder}/weg.bin.se-upload-1111111111111111");
    let mut writer = backend.open_write_copy_stage_unsynced(&dropped, 5).unwrap();
    writer.write_all(b"he").unwrap();
    drop(writer);
    assert_eq!(kind(backend.stat(&dropped)), Some(ErrorKind::NotFound));
    let short = format!("{folder}/kurz.bin.se-upload-2222222222222222");
    let mut writer = backend.open_write_copy_stage_unsynced(&short, 5).unwrap();
    writer.write_all(b"hel").unwrap();
    assert!(writer.flush().is_err());
    drop(writer);
    assert_eq!(kind(backend.stat(&short)), Some(ErrorKind::NotFound));

    // Reads: inside, exactly at and beyond one READ block (512 KiB), whole
    // and as ranges; an empty file.
    for len in [0usize, 1, 512 * 1024, 512 * 1024 + 1, 3 * 1024 * 1024 + 7] {
        let path = format!("{folder}/lesen-{len}.bin");
        let bytes = content(len, len as u8);
        write_new(&*backend, &path, &bytes);
        assert!(read_path(&*backend, &path) == bytes, "{len}");
        if len > 10 {
            let range = backend.open_read_range(&path, None, 3, len as u64 - 5);
            let mut range = range.unwrap().expect("SMB reads ranges");
            assert!(read_all(&mut *range) == bytes[3..len - 2], "range of {len}");
            let mut tail = backend.open_read_at(&path, None, 7).unwrap().unwrap();
            assert!(read_all(&mut *tail) == bytes[7..], "tail of {len}");
        }
    }
    cleanup(&*backend, &folder);
}

/// Best effort: removes the case folder's files and the folder.
fn cleanup(backend: &dyn Backend, folder: &str) {
    if let Ok(entries) = backend.list_dir(folder) {
        for entry in entries {
            let _ = backend.remove_file(&format!("{folder}/{}", entry.name));
        }
    }
    let _ = backend.remove_dir(folder);
}

#[test]
#[ignore = "benchmark: needs the task suite's Nextcloud"]
fn speed_task_remote_nextcloud_chunked_large_byte_exact() {
    let Some((backend, root)) = nextcloud() else {
        return;
    };
    let source = local_case(&LARGE, "nextcloud_chunked");
    let remote = remote_folder(&*backend, &root, "bench-chunked-large");
    let up = bench_job(
        false,
        job(
            Endpoint::Local,
            Endpoint::Remote(backend.clone()),
            vec![slash(&source.join(TOP))],
            remote.clone(),
            "bench",
        ),
    );
    report("nextcloud_chunked_large_up_plain", &up, 1);
    let back = local_folder("bench-back-nextcloud-chunked");
    let down = bench_job(
        false,
        job(
            Endpoint::Remote(backend.clone()),
            Endpoint::Local,
            vec![format!("{remote}/{TOP}")],
            slash(&back),
            "bench",
        ),
    );
    report("nextcloud_chunked_large_down_plain", &down, 1);
    let file = Path::new(TOP).join("d0").join("f0.bin");
    assert!(
        same_bytes(&source.join(&file), &back.join(&file)),
        "the chunks assembled the file byte for byte"
    );
    let _ = std::fs::remove_dir_all(&back);
    let _ = std::fs::remove_dir_all(&source);
}

/// Server-side copies of a packet in one go (`vfs::server_copy_batch`):
/// every member lands byte for byte in its own new stage; a taken stage
/// name is `AlreadyExists` and keeps what is there.
#[test]
#[ignore = "needs the task suite's SFTP server"]
fn speed_task_remote_sftp_server_copy_batch() {
    let (backend, root) = crate::support::sftp();
    let folder = remote_folder(&*backend, &root, "copy-batch");
    let mut items = Vec::new();
    let mut contents = Vec::new();
    for index in 0..100usize {
        let source = format!("{folder}/q{index}.bin");
        let bytes = content(index * 37, index as u8);
        write_new(&*backend, &source, &bytes);
        let stage = format!("{folder}/z{index}.bin.se-upload-{index:016x}");
        items.push((source, stage, bytes.len() as u64));
        contents.push(bytes);
    }
    let taken = format!("{folder}/belegt.bin");
    write_new(&*backend, &taken, b"theirs");
    items.push((items[1].0.clone(), taken.clone(), contents[1].len() as u64));

    let started = std::time::Instant::now();
    let answers = vfs::server_copy_batch(&*backend, &items);
    let seconds = started.elapsed().as_secs_f64().max(f64::EPSILON);
    assert_eq!(answers.len(), items.len());
    if answers.iter().all(|answer| matches!(answer, Ok(None))) {
        println!("SKIP the SFTP server offers no copy-data");
        cleanup(&*backend, &folder);
        return;
    }
    println!(
        "BENCH sftp_server_copy_batch files_per_s={:.1}",
        100.0 / seconds
    );
    for ((_, stage, size), (answer, bytes)) in items.iter().zip(answers.iter().zip(&contents)) {
        assert_eq!(
            answer.as_ref().ok().copied().flatten(),
            Some(*size),
            "{stage}"
        );
        assert!(read_path(&*backend, stage) == *bytes, "{stage}");
    }
    let last = answers.last().expect("one answer per item");
    let refused = last.as_ref().map_err(|error| error.kind());
    assert_eq!(refused.err(), Some(ErrorKind::AlreadyExists));
    assert_eq!(read_path(&*backend, &taken), b"theirs");
    cleanup(&*backend, &folder);
}

/// Same-account packets through the engine (G3-13/R2): an SFTP → same SFTP
/// copy of many small files goes as packets of server copies
/// (`vfs::server_copy_batch`). Every file must land byte for byte, a name
/// that exists at the target is never replaced (the copy is numbered), and
/// no private stage is left behind. Without `copy-data` the files stream
/// alone and the same must hold.
#[test]
#[ignore = "needs the task suite's SFTP server"]
fn speed_task_remote_sftp_same_account_packets_byte_exact_never_replace() {
    let (backend, root) = crate::support::sftp();
    let folder = remote_folder(&*backend, &root, "same-account-packets");
    let source = format!("{folder}/src/Vault");
    backend.mkdir_all(&source).expect("source folder");
    let mut expected = std::collections::BTreeMap::new();
    for index in 0..150usize {
        let name = format!("f{index:03}.bin");
        let bytes = content(index * 53 + 1, index as u8);
        write_new(&*backend, &format!("{source}/{name}"), &bytes);
        expected.insert(name, Some(bytes));
    }
    // The target already holds one of the names: it must stay theirs.
    let target = format!("{folder}/dst");
    backend
        .mkdir_all(&format!("{target}/Vault"))
        .expect("target");
    write_new(&*backend, &format!("{target}/Vault/f007.bin"), b"theirs");
    let ours = expected
        .insert("f007.bin".to_string(), Some(b"theirs".to_vec()))
        .expect("listed");
    expected.insert("f007 (2).bin".to_string(), ours);

    let started = std::time::Instant::now();
    let measured = crate::support::run(
        job(
            Endpoint::Remote(backend.clone()),
            Endpoint::Remote(backend.clone()),
            vec![source.clone()],
            target.clone(),
            "same-account",
        ),
        std::time::Duration::from_secs(600),
    );
    let seconds = started.elapsed().as_secs_f64().max(f64::EPSILON);
    println!(
        "BENCH sftp_same_account_packets files_per_s={:.1}",
        150.0 / seconds
    );
    assert_eq!(measured.files, 150);
    let landed = crate::support::remote_tree(&*backend, &format!("{target}/Vault"));
    let stages: Vec<&String> = landed
        .keys()
        .filter(|name| name.contains(".se-upload-"))
        .collect();
    assert!(stages.is_empty(), "stages left behind: {stages:?}");
    assert!(
        landed == expected,
        "every file byte for byte, none replaced"
    );
    cleanup_tree(&*backend, &folder);
}

/// Best effort: removes a case folder with its subfolders.
fn cleanup_tree(backend: &dyn Backend, folder: &str) {
    if let Ok(entries) = backend.list_dir(folder) {
        for entry in entries {
            let path = format!("{folder}/{}", entry.name);
            if entry.is_dir {
                cleanup_tree(backend, &path);
            } else {
                let _ = backend.remove_file(&path);
            }
        }
    }
    let _ = backend.remove_dir(folder);
}
