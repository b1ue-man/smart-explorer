//! The MD5s of many files of a plain SFTP server through one exec request
//! per group (digest_batch.rs, digest_batch_flow.rs): the fixed script keeps
//! the rules of the single-file script, hashes many paths correctly in every
//! installed login shell (names with spaces, newlines, quotes, `$()`,
//! backslashes, a leading `-`), answers "cannot form" for exactly the path
//! that is missing, and the flow sends one exec per group and checks size
//! and time of every file before and after.
use super::super::digest_batch_flow::{md5_many_by, Gates};
use super::super::digest_command::DigestGate;
use super::*;
use crate::vfs::VfsMeta;
use std::cell::{Cell, RefCell};
use std::io;
use std::sync::atomic::AtomicBool;

fn captured(status: Option<u32>, stdout: &str) -> CapturedExec {
    CapturedExec {
        stdout: stdout.as_bytes().to_vec(),
        exit_status: status,
        ..CapturedExec::default()
    }
}

const MD5_A: &str = "0cc175b9c0f1b6a831c399e269772661";
const MD5_B: &str = "92eb5ffee6ae2fec3ad71c777531578f";

#[test]
fn abgleich_task_sftp_batch_script_is_fixed_and_carries_no_marker() {
    for forbidden in ['\'', '\\', '!', '\n', '\r'] {
        assert!(!MD5_BATCH_SCRIPT.contains(forbidden), "{forbidden:?}");
    }
    assert!(
        !MD5_BATCH_SCRIPT.contains(MARKER),
        "the marker is built at run time"
    );
    assert!(MD5_BATCH_SCRIPT.contains("set -f; LC_ALL=C"));
    assert!(MD5_BATCH_SCRIPT.contains("wc -c -- \"$@\""));
    assert!(MD5_BATCH_SCRIPT.contains("$h -- \"$@\""));
    assert!(!MD5_BATCH_SCRIPT.contains("openssl"));
    assert_eq!(batch_command_line(), format!("sh -c '{MD5_BATCH_SCRIPT}'"));
}

#[test]
fn abgleich_task_sftp_batch_groups_send_paths_only_on_standard_input() {
    let files = [
        (0, "/a b", 1),
        (1, "/line\nbreak", 2),
        (2, "", 3),
        (3, "/nul\0x", 4),
        (4, "/q\"u'o", 5),
    ];
    let found = groups(&files);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].members,
        vec![0, 1, 4],
        "empty and NUL paths go alone"
    );
    assert_eq!(found[0].bytes, 1 + 2 + 5);
    let separator = found[0].stdin[1];
    assert_eq!(found[0].stdin[0], b'P');
    assert!(separators().any(|byte| byte == separator));
    let mut expected = vec![b'P', separator];
    expected.extend_from_slice(b"m/a b");
    expected.push(separator);
    expected.extend_from_slice(b"o/line\nbreak");
    expected.push(separator);
    expected.extend_from_slice(b"m/q\"u'o");
    assert_eq!(found[0].stdin, expected);

    // The separator is in no path of its group; a path holding every
    // candidate goes alone.
    let every: String = separators().map(char::from).collect();
    let with_first = format!("/x{}", char::from(separators().next().unwrap()));
    let tricky = [(0, with_first.as_str(), 1), (1, every.as_str(), 1)];
    let found = groups(&tricky);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].members, vec![0]);
    assert!(!with_first.as_bytes().contains(&found[0].stdin[1]));
}

#[test]
fn abgleich_task_sftp_batch_groups_respect_count_output_and_content_limits() {
    let names: Vec<String> = (0..3000).map(|index| format!("/srv/f{index}")).collect();
    let files: Vec<(usize, &str, u64)> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (index, name.as_str(), 10))
        .collect();
    let found = groups(&files);
    assert!(found.len() >= 3, "{}", found.len());
    let members: Vec<usize> = found
        .iter()
        .flat_map(|group| group.members.clone())
        .collect();
    assert_eq!(
        members,
        (0..3000).collect::<Vec<_>>(),
        "every file once, in order"
    );
    assert!(found.iter().all(|group| group.members.len() <= MAX_FILES));
    let large = [
        (0, "/a", MAX_CONTENT),
        (1, "/b", 1),
        (2, "/c", MAX_CONTENT + 1),
    ];
    let found = groups(&large);
    let members: Vec<Vec<usize>> = found.iter().map(|group| group.members.clone()).collect();
    assert_eq!(members, vec![vec![0], vec![1], vec![2]]);
}

#[test]
fn abgleich_task_sftp_batch_reads_every_tool_output() {
    let gnu = format!(
        "motd line\n{MARKER} file\n{MARKER} none\n{MARKER} one 7 \\{MD5_B}  -\n{MARKER} fail\n\
         {MARKER} file\n{MARKER} sizes\n  1 /a\n 12 /b c\n 13 total\n{MARKER} digests\n\
         {MD5_A}  /a\n\\{}  /b\\\\c\n{MARKER} end\n",
        MD5_B.to_ascii_uppercase()
    );
    assert_eq!(
        interpret_batch(&captured(Some(0), &gnu), 5),
        BatchReply::Answers {
            answers: vec![
                Answer::Digest {
                    hex: MD5_A.into(),
                    size: 1
                },
                Answer::NotRegular,
                Answer::Digest {
                    hex: MD5_B.into(),
                    size: 7
                },
                Answer::Missing,
                Answer::Digest {
                    hex: MD5_B.into(),
                    size: 12
                },
            ],
            aligned: true
        }
    );
    // BSD `wc` pads, `md5 -r` prints one space; one file has no total.
    let bsd = format!(
        "{MARKER} file\n{MARKER} sizes\n       1 /a\n{MARKER} digests\n{MD5_A} /a\n{MARKER} end\n"
    );
    assert_eq!(
        interpret_batch(&captured(Some(0), &bsd), 1),
        BatchReply::Answers {
            answers: vec![Answer::Digest {
                hex: MD5_A.into(),
                size: 1
            }],
            aligned: true
        }
    );
}

#[test]
fn abgleich_task_sftp_batch_never_pairs_a_section_that_does_not_line_up() {
    // The second file vanished between the test and the tools.
    let short = format!(
        "{MARKER} file\n{MARKER} file\n{MARKER} sizes\n1 /a\n1 total\n{MARKER} digests\n\
         {MD5_A}  /a\n{MARKER} end\n"
    );
    assert_eq!(
        interpret_batch(&captured(Some(0), &short), 2),
        BatchReply::Answers {
            answers: vec![Answer::Missing, Answer::Missing],
            aligned: false
        }
    );
    // Another number of answers than paths: the shell split otherwise.
    let split = format!("{MARKER} none\n{MARKER} end\n");
    assert!(matches!(
        interpret_batch(&captured(Some(0), &split), 2),
        BatchReply::Failed(_)
    ));
    let unfinished = format!("{MARKER} file\n{MARKER} sizes\n1 /a\n");
    assert!(matches!(
        interpret_batch(&captured(Some(0), &unfinished), 1),
        BatchReply::Failed(_)
    ));
}

#[test]
fn abgleich_task_sftp_batch_without_the_marker_or_a_tool_is_unavailable() {
    for (status, stdout) in [
        (Some(11), String::new()),
        (Some(0), String::new()),
        (
            Some(1),
            "This service allows sftp connections only.\n".to_string(),
        ),
        (
            Some(9009),
            "'sh' is not recognized as a command\n".to_string(),
        ),
        (Some(0), format!("{MARKER} notool\n")),
    ] {
        assert_eq!(
            interpret_batch(&captured(status, &stdout), 3),
            BatchReply::Unavailable,
            "{status:?} {stdout:?}"
        );
    }
    assert!(matches!(
        interpret_batch(&captured(None, ""), 1),
        BatchReply::Failed(_)
    ));
}

fn listed(size: u64, mtime_ms: i64) -> VfsMeta {
    VfsMeta {
        size,
        mtime_ms,
        name: "f".into(),
        ..VfsMeta::default()
    }
}

fn gates<'a>(md5: &'a DigestGate, batch: &'a DigestGate) -> Gates<'a> {
    Gates { md5, batch }
}

/// What the script prints for `count` regular files of `size` bytes.
fn all_files(count: usize, size: u64) -> String {
    let mut out = String::new();
    for _ in 0..count {
        out.push_str(&format!("{MARKER} file\n"));
    }
    out.push_str(&format!("{MARKER} sizes\n"));
    for index in 0..count {
        out.push_str(&format!("{size} /f{index}\n"));
    }
    if count > 1 {
        out.push_str(&format!("{} total\n", size * count as u64));
    }
    out.push_str(&format!("{MARKER} digests\n"));
    for index in 0..count {
        out.push_str(&format!("{MD5_A}  /f{index}\n"));
    }
    out.push_str(&format!("{MARKER} end\n"));
    out
}

#[test]
fn abgleich_task_sftp_batch_flow_checks_size_and_time_before_and_after() {
    let (md5, batch) = (DigestGate::default(), DigestGate::default());
    let metas = [listed(1, 1_000), listed(1, 1_000), listed(1, 1_000)];
    let files: Vec<(&str, &VfsMeta)> =
        vec![("/f0", &metas[0]), ("/f1", &metas[1]), ("/f2", &metas[2])];
    let stats = Cell::new(0);
    let runs = RefCell::new(Vec::new());
    let found = md5_many_by(
        gates(&md5, &batch),
        &files,
        &AtomicBool::new(false),
        |paths| {
            stats.set(stats.get() + 1);
            paths
                .iter()
                .map(|path| match (stats.get(), *path) {
                    // Changed since the listing: not sent.
                    (1, "/f2") => Some(listed(2, 1_000)),
                    // Changed while it was hashed.
                    (2, "/f1") => Some(listed(1, 2_000)),
                    _ => Some(listed(1, 1_000)),
                })
                .collect()
        },
        |command, stdin, _| {
            runs.borrow_mut()
                .push((command.to_string(), stdin.to_vec()));
            Ok(Some(captured(Some(0), &all_files(2, 1))))
        },
    )
    .unwrap();
    assert_eq!(found, vec![Some(MD5_A.to_string()), None, None]);
    assert_eq!(stats.get(), 2, "one pipelined round before, one after");
    let runs = runs.into_inner();
    assert_eq!(runs.len(), 1, "one exec for the group");
    assert_eq!(runs[0].0, batch_command_line(), "the fixed text, no path");
    assert!(!md5.closed() && !batch.closed());
}

#[test]
fn abgleich_task_sftp_batch_flow_closes_the_md5_gate_where_the_shell_sees_otherwise() {
    let metas = [listed(5, 1_000), listed(1, 1_000)];
    let files: Vec<(&str, &VfsMeta)> = vec![("/f0", &metas[0]), ("/f1", &metas[1])];
    let same = |paths: &[&str]| -> Vec<Option<VfsMeta>> {
        paths
            .iter()
            .map(|path| {
                Some(if *path == "/f0" {
                    listed(5, 1_000)
                } else {
                    listed(1, 1_000)
                })
            })
            .collect()
    };
    // The shell read another size for an SFTP-unchanged file.
    let (md5, batch) = (DigestGate::default(), DigestGate::default());
    let found = md5_many_by(
        gates(&md5, &batch),
        &files,
        &AtomicBool::new(false),
        same,
        |_, _, _| Ok(Some(captured(Some(0), &all_files(2, 1)))),
    )
    .unwrap();
    assert_eq!(found, vec![None, None]);
    assert!(md5.closed(), "this connection's shell view is not SFTP's");

    // No marker (cmd.exe, a forced `internal-sftp`), a refused exec.
    for refused in [false, true] {
        let (md5, batch) = (DigestGate::default(), DigestGate::default());
        let runs = Cell::new(0);
        let found = md5_many_by(
            gates(&md5, &batch),
            &files,
            &AtomicBool::new(false),
            same,
            |_, _, _| {
                runs.set(runs.get() + 1);
                if refused {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok(Some(captured(Some(11), "")))
                }
            },
        )
        .unwrap();
        assert_eq!(found, vec![None, None]);
        assert!(md5.closed());
        assert_eq!(runs.get(), 1);
        // A closed gate looks at nothing and sends nothing.
        let again = md5_many_by(
            gates(&md5, &batch),
            &files,
            &AtomicBool::new(false),
            |_| panic!("a closed gate looks at nothing"),
            |_, _, _| panic!("a closed gate sends nothing"),
        )
        .unwrap();
        assert_eq!(again, vec![None, None]);
    }
}

#[test]
fn abgleich_task_sftp_batch_flow_answers_that_do_not_line_up_close_only_the_batch_way() {
    let (md5, batch) = (DigestGate::default(), DigestGate::default());
    let metas = [listed(1, 1_000), listed(1, 1_000)];
    let files: Vec<(&str, &VfsMeta)> = vec![("/f0", &metas[0]), ("/f1", &metas[1])];
    let short = format!(
        "{MARKER} file\n{MARKER} file\n{MARKER} sizes\n1 /a\n1 total\n{MARKER} digests\n\
         {MD5_A}  /a\n{MARKER} end\n"
    );
    for _ in 0..3 {
        let found = md5_many_by(
            gates(&md5, &batch),
            &files,
            &AtomicBool::new(false),
            |paths| paths.iter().map(|_| Some(listed(1, 1_000))).collect(),
            |_, _, _| Ok(Some(captured(Some(0), &short))),
        )
        .unwrap();
        assert_eq!(found, vec![None, None]);
    }
    assert!(batch.closed(), "three in a row: the batch way stops");
    assert!(!md5.closed(), "the single-file way stays");
}

#[cfg(unix)]
#[path = "abgleich_task_digest_batch_shell_tests.rs"]
mod shells;
