//! The sync's MD5 of many files of a plain SFTP server through one exec
//! request instead of one per file (`content_md5_many`, the checksum compare
//! mode): opening a session channel and starting a shell costs more time and
//! bytes than a small file itself, and a shell that starts `wc` and `md5sum`
//! once per file spends milliseconds of the server's time on every file.
//! The script here runs `wc -c` and `md5sum` (or `md5 -r`) once for all
//! files of a group.
//!
//! The rules of `digest_command.rs` stay: one fixed command line
//! `sh -c '<script>'` without a path, a single quote, backslash, `!` or
//! newline, so every login shell (sh, bash, dash, zsh, ksh, fish, csh, tcsh)
//! passes the script literally; the paths travel on standard input after the
//! byte `P` (an SFTP server that a forced command put on the channel reads a
//! length far above 256 KiB and ends at once); every answer line starts with
//! the marker the script builds at run time, so cmd.exe, PowerShell or a
//! forced command cannot print it; the size the shell read is checked per
//! file against the SFTP attributes.
//!
//! Standard input: `P`, one separator byte, then the entries joined by that
//! byte, each a flag byte and the path. The separator is a control byte that
//! is in no path of the group (`SEPARATORS`), so the shell's field splitting
//! (`IFS` set to it, globbing off, `LC_ALL=C` so every byte is one character)
//! gives back exactly the paths. The flag `m` puts the path into the group
//! run of `wc` and `md5sum`; `o` (a path with a newline, whose name the tools
//! may print on two lines) is hashed on its own through redirects, as the
//! single-file script does.
//!
//! Output: one classification line per entry in input order (`file`, `none`
//! for a link or no regular file, `fail` for an unreadable one, `one <size>
//! <digest>` for an `o` entry), then `sizes` with the lines of `wc -c`, then
//! `digests` with the lines of the MD5 tool, then `end`. The tools print one
//! line per file in argument order (a file they cannot read gets none) and
//! `wc` a total after more than one; only the first word of a tool line is
//! read, so the way a tool quotes a name does not matter. A section whose
//! line count is not the number of `file` entries proves nothing about any of
//! them (`Answer::Missing`), never a wrong pairing.
use super::digest_command::{MARKER, STDIN_PREFIX};
use super::exec::CapturedExec;

/// The fixed script; the paths come on standard input.
pub(super) const MD5_BATCH_SCRIPT: &str = concat!(
    "set -f; LC_ALL=C; export LC_ALL; a=se-; m=${a}digest-v1; ",
    "p=$(cat; echo x); p=${p%x}; p=${p#P}; s=${p%\"${p#?}\"}; p=${p#?}; ",
    "if command -v md5sum >/dev/null 2>&1; then h=md5sum; ",
    "elif command -v md5 >/dev/null 2>&1; then h=\"md5 -r\"; ",
    "else echo \"$m\" notool; exit 0; fi; ",
    "IFS=$s; set -- $p; IFS=\" \"; k=; n=0; ",
    "for e in \"$@\"; do f=${e#?}; ",
    "if [ -h \"$f\" ]; then echo \"$m\" none; ",
    "elif [ -f \"$f\" ]; then if [ -r \"$f\" ]; then case $e in ",
    "o*) z=$(wc -c <\"$f\") && d=$($h <\"$f\") && echo \"$m\" one $z $d || echo \"$m\" fail;; ",
    "*) echo \"$m\" file; k=$k$s$f; n=$((n+1));; esac; ",
    "else echo \"$m\" fail; fi; ",
    "else echo \"$m\" none; fi; done; ",
    "if [ $n -gt 0 ]; then IFS=$s; set -- ${k#?}; IFS=\" \"; ",
    "echo \"$m\" sizes; wc -c -- \"$@\"; echo \"$m\" digests; $h -- \"$@\"; fi; ",
    "echo \"$m\" end"
);

/// Most files in one group: the script rebuilds its argument list per file.
const MAX_FILES: usize = 1024;
/// The output of a group stays below the 64 KiB the exec capture keeps.
const OUTPUT_BUDGET: usize = 56 * 1024;
/// Content of one group; a larger file forms a group of its own. Bounds the
/// wait of one exec (`digest_command::wait_for`).
const MAX_CONTENT: u64 = 256 * 1024 * 1024;
/// Output lines besides the names: classification, `wc`, digest line.
const LINE_OVERHEAD: usize = 96;

/// The fixed command line of the group script.
pub(super) fn batch_command_line() -> String {
    format!("sh -c '{MD5_BATCH_SCRIPT}'")
}

/// Control bytes that may separate the paths: no NUL (no shell variable holds
/// it), no 0x01 (bash's internal quoting byte), no tab or newline (field
/// splitting treats them as white space).
pub(super) fn separators() -> impl Iterator<Item = u8> {
    (2u8..0x20).filter(|byte| *byte != b'\t' && *byte != b'\n')
}

/// One exec request: which of the caller's files it carries and what goes
/// to its standard input.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Group {
    /// Indices into the caller's list, in input order.
    pub(super) members: Vec<usize>,
    pub(super) stdin: Vec<u8>,
    /// Content bytes of the members (the wait of the exec).
    pub(super) bytes: u64,
}

#[derive(Default)]
struct Building {
    members: Vec<usize>,
    paths: Vec<usize>,
    present: u32,
    output: usize,
    bytes: u64,
}

fn control_bytes(path: &str) -> u32 {
    path.bytes()
        .filter(|byte| *byte < 0x20)
        .fold(0, |set, byte| set | 1 << byte)
}

fn free_separator(present: u32) -> Option<u8> {
    separators().find(|byte| present & (1 << byte) == 0)
}

/// Splits `files` (index, path, size) into groups. A path that cannot be
/// sent (empty, a NUL, every separator in it) is in no group: the caller
/// asks for it on its own.
pub(super) fn groups(files: &[(usize, &str, u64)]) -> Vec<Group> {
    let mut done = Vec::new();
    let mut open = Building::default();
    for (position, (index, path, size)) in files.iter().enumerate() {
        if path.is_empty() || path.contains('\0') {
            continue;
        }
        let own = control_bytes(path);
        if free_separator(own).is_none() {
            continue;
        }
        let output = 2 * path.len() + LINE_OVERHEAD;
        let full = !open.members.is_empty()
            && (open.members.len() >= MAX_FILES
                || open.output + output > OUTPUT_BUDGET
                || open.bytes.saturating_add(*size) > MAX_CONTENT
                || free_separator(open.present | own).is_none());
        if full {
            done.push(finish(std::mem::take(&mut open), files));
        }
        open.members.push(*index);
        open.paths.push(position);
        open.present |= own;
        open.output += output;
        open.bytes = open.bytes.saturating_add(*size);
    }
    if !open.members.is_empty() {
        done.push(finish(open, files));
    }
    done
}

fn finish(group: Building, files: &[(usize, &str, u64)]) -> Group {
    let separator = free_separator(group.present).unwrap_or(0x1f);
    let mut stdin = vec![STDIN_PREFIX, separator];
    for (count, position) in group.paths.iter().enumerate() {
        let path = files[*position].1;
        if count > 0 {
            stdin.push(separator);
        }
        stdin.push(if path.contains('\n') { b'o' } else { b'm' });
        stdin.extend_from_slice(path.as_bytes());
    }
    Group {
        members: group.members,
        stdin,
        bytes: group.bytes,
    }
}

/// What the script said about one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    /// The MD5 and the size the shell read.
    Digest { hex: String, size: u64 },
    /// The shell saw a link or no regular file at the path.
    NotRegular,
    /// No usable answer for this entry (unreadable, a tool failed, a section
    /// that did not line up).
    Missing,
}

/// What the finished group command means.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum BatchReply {
    /// One answer per entry; `aligned` is false when a tool section did not
    /// line up with the `file` entries (a file vanished between the test and
    /// the tool, or a tool that prints otherwise).
    Answers { answers: Vec<Answer>, aligned: bool },
    /// This connection will never answer: the text did not run as the
    /// script (no marker), or the host has no MD5 tool.
    Unavailable,
    /// This time it did not work.
    Failed(String),
}

#[derive(Clone, Copy)]
enum Section {
    Classes,
    Sizes,
    Digests,
    Ended,
}

/// Reads the output of a group of `count` entries.
pub(super) fn interpret_batch(capture: &CapturedExec, count: usize) -> BatchReply {
    if capture.exit_signal.is_some() {
        return BatchReply::Failed("the hash command was ended by a signal".into());
    }
    let lines: Vec<String> = capture
        .stdout
        .split(|byte| *byte == b'\n')
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect();
    let marked = |line: &str| line.split_whitespace().next() == Some(MARKER);
    if !lines.iter().any(|line| marked(line.as_str())) {
        return match capture.exit_status {
            None if !capture.stdout_truncated => {
                BatchReply::Failed("the hash command gave no exit status".into())
            }
            _ => BatchReply::Unavailable,
        };
    }
    if capture.stdout_truncated {
        return BatchReply::Failed("the hash command printed too much".into());
    }
    if capture.exit_status != Some(0) {
        return BatchReply::Failed(format!(
            "the hash script ended with status {:?}",
            capture.exit_status
        ));
    }
    let mut answers = Vec::new();
    let mut files = Vec::new();
    let (mut sizes, mut digests) = (Vec::new(), Vec::new());
    let mut section = Section::Classes;
    for line in &lines {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if words[0] == MARKER {
            match (section, &words[1..]) {
                (_, ["notool"]) => return BatchReply::Unavailable,
                (Section::Classes, ["file"]) => {
                    files.push(answers.len());
                    answers.push(Answer::Missing);
                }
                (Section::Classes, ["none"]) => answers.push(Answer::NotRegular),
                (Section::Classes, ["fail"]) => answers.push(Answer::Missing),
                (Section::Classes, ["one", size, digest, ..]) => {
                    match (size.parse::<u64>(), md5_hex(digest)) {
                        (Ok(size), Some(hex)) => answers.push(Answer::Digest { hex, size }),
                        _ => return BatchReply::Unavailable,
                    }
                }
                (Section::Classes, ["sizes"]) => section = Section::Sizes,
                (Section::Sizes, ["digests"]) => section = Section::Digests,
                (Section::Classes | Section::Digests, ["end"]) => section = Section::Ended,
                _ => return BatchReply::Failed(format!("unexpected answer {line:?}")),
            }
            continue;
        }
        // Output of a profile before the script ran is not the script's.
        match section {
            Section::Classes => {}
            Section::Sizes => sizes.push(words[0].to_string()),
            Section::Digests => digests.push(words[0].to_string()),
            Section::Ended => return BatchReply::Failed(format!("output after the end {line:?}")),
        }
    }
    if !matches!(section, Section::Ended) {
        return BatchReply::Failed("the hash script did not finish".into());
    }
    if answers.len() != count {
        return BatchReply::Failed(format!("{} answers for {count} paths", answers.len()));
    }
    let aligned = pair(&mut answers, &files, &sizes, &digests);
    BatchReply::Answers { answers, aligned }
}

/// Gives the `file` entries their size and digest when both sections line
/// up with them; otherwise they stay `Missing`.
fn pair(answers: &mut [Answer], files: &[usize], sizes: &[String], digests: &[String]) -> bool {
    if files.is_empty() {
        return sizes.is_empty() && digests.is_empty();
    }
    let total = usize::from(files.len() > 1);
    if sizes.len() != files.len() + total || digests.len() != files.len() {
        return false;
    }
    let parsed: Option<Vec<(u64, String)>> = sizes
        .iter()
        .zip(digests)
        .map(|(size, digest)| Some((size.parse::<u64>().ok()?, md5_hex(digest)?)))
        .collect();
    let Some(parsed) = parsed else {
        return false;
    };
    for (entry, (size, hex)) in files.iter().zip(parsed) {
        answers[*entry] = Answer::Digest { hex, size };
    }
    true
}

/// The first word of an MD5 line (GNU marks an escaped name with a leading
/// backslash) as lowercase hex, when it is one.
fn md5_hex(word: &str) -> Option<String> {
    let hex = word.trim_start_matches('\\').to_ascii_lowercase();
    (hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then_some(hex)
}

#[cfg(test)]
#[path = "abgleich_task_digest_batch_tests.rs"]
mod tests;
