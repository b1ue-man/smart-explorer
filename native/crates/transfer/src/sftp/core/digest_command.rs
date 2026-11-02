//! `content_digest` of a plain SFTP server (no SSH agent next to the data):
//! the SSH server hashes the file itself and sends the hex digest, not the
//! content (RFC 4254 section 6.5 exec request on a session channel of the
//! connection that already carries the SFTP subsystem). Only when the account
//! may run commands and the host has `sha256sum` (GNU coreutils, busybox),
//! `shasum -a 256` (Perl, macOS) or `sha256 -q` (BSD); otherwise the answer
//! is `Ok(None)` and the caller moves the content as before.
//!
//! The command line is one fixed text, `sh -c '<script>'`, the same for
//! every file: no path, name or other byte from the user or the file system
//! is in it. sshd hands the text to the account's login shell
//! (`$SHELL -c <text>`, OpenSSH `session.c` `do_child`), and the script
//! holds no single quote, backslash, `!` or newline, so every login shell
//! reads the quoted part literally: POSIX sh, bash, dash and ksh (POSIX.1-2024
//! XCU 2.2.2 "Single-Quotes"), zsh (manual 6.9 "Quoting"; `RC_QUOTES` only
//! changes `''`, which the text does not hold), fish ("Quotes": only `\'`
//! and `\\` are escapes in single quotes) and csh/tcsh (tcsh(1) "Lexical
//! structure": single quotes stop every substitution except history `!`).
//!
//! The path travels on the channel's standard input after the request: the
//! byte `P`, the path's bytes, then EOF. The script reads all of it
//! (`$(cat; echo x)` keeps trailing newlines), drops the `P` and reads the
//! file through a redirect, so neither the path nor its first characters
//! can be taken for an option or a command. Only a path with a NUL (no POSIX
//! path holds one) is not sent. The `P` makes the first four bytes a length
//! far above 256 KiB for an SFTP server that a forced command
//! (`ForceCommand internal-sftp` of older servers, an external `sftp-server`)
//! put on the channel: OpenSSH's `sftp-server.c` `process()` then exits with
//! status 11 ("bad message"), and a shorter input ends at the EOF ("read
//! eof", status 0). Neither waits for the time limit.
//!
//! Every answer of the script is one line that starts with a marker the
//! script builds at run time (`a=se-; m=${a}digest-v1`), so the marker is
//! not in the command line: a shell that did not run the text as POSIX sh
//! (Windows OpenSSH with cmd.exe or PowerShell, a restricted or forced
//! command that prints the original command) cannot print it. No marker means
//! "cannot hash here" (`Ok(None)`, the gate closes), never "file changed".
//! The script also reports the size it read (`wc -c`); a size other than the
//! SFTP attributes' while those stay the same means the shell sees another
//! file under that path (another root or chroot) and the gate closes. Only
//! SFTP attributes that differ before and after the command (size or time)
//! mean the file changed while it was hashed; that is an error, never a
//! digest.
//!
//! The sync's MD5 signature (`content_md5`) goes the same way with a second
//! fixed script (`MD5_SCRIPT`): `md5sum` (GNU coreutils, busybox) or
//! `md5 -q` (BSD, macOS). No other tool is tried (no `openssl`), and a host
//! with neither answers `Ok(None)`. Each algorithm has its own gate: a host
//! without `sha256sum` may still have `md5sum`. The checksum compare mode asks
//! for the MD5s of a folder's files together (`content_md5_many`,
//! digest_batch.rs): one exec request per group, under the same rules and
//! the same MD5 gate.
use super::backend::SftpBackend;
use super::exec::CapturedExec;
use crate::vfs::{is_sha256_hex, ContentDigest, VfsMeta, VfsResult};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The first word of every line the script answers with. The script joins
/// it from two parts, so the command line never holds it.
pub(super) const MARKER: &str = "se-digest-v1";
/// The byte before the path on standard input.
pub(super) const STDIN_PREFIX: u8 = b'P';
/// Failures in a row (none of them proving that commands are unavailable)
/// after which the connection stops trying.
const FAILURES_BEFORE_GIVING_UP: u32 = 3;
/// A host that hashes slower than this is not worth waiting for: the
/// content would arrive sooner over the connection.
const SLOWEST_HASHING: u64 = 8 * 1024 * 1024;
/// Time to start the command and answer for a small file.
const BASE_WAIT: Duration = Duration::from_secs(60);

/// The fixed script around the tool choice `$tools` (which sets `h` or
/// answers `notool`). No single quote, backslash, `!` or newline.
macro_rules! digest_script {
    ($tools:literal) => {
        concat!(
            "set -f; a=se-; m=${a}digest-v1; p=$(cat; echo x); p=${p%x}; f=${p#P}; ",
            "[ -h \"$f\" ] && { echo \"$m\" none; exit 0; }; ",
            "[ -f \"$f\" ] || { echo \"$m\" none; exit 0; }; ",
            $tools,
            "s=$(wc -c <\"$f\") || { echo \"$m\" fail; exit 0; }; ",
            "d=$($h <\"$f\") || { echo \"$m\" fail; exit 0; }; ",
            "echo \"$m\" ok $s $d"
        )
    };
}

/// The fixed SHA-256 script; the path comes on standard input.
pub(super) const SCRIPT: &str = digest_script!(
    "if command -v sha256sum >/dev/null 2>&1; then h=sha256sum; \
elif command -v shasum >/dev/null 2>&1; then h=\"shasum -a 256\"; \
elif command -v sha256 >/dev/null 2>&1; then h=\"sha256 -q\"; \
else echo \"$m\" notool; exit 0; fi; "
);

/// The fixed MD5 script; same rules as `SCRIPT`.
pub(super) const MD5_SCRIPT: &str = digest_script!(
    "if command -v md5sum >/dev/null 2>&1; then h=md5sum; \
elif command -v md5 >/dev/null 2>&1; then h=\"md5 -q\"; \
else echo \"$m\" notool; exit 0; fi; "
);

/// What the host is asked to form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Algorithm {
    Sha256,
    Md5,
}

impl Algorithm {
    pub(super) fn script(self) -> &'static str {
        match self {
            Self::Sha256 => SCRIPT,
            Self::Md5 => MD5_SCRIPT,
        }
    }

    /// Whether `text` is a digest of this algorithm in lowercase hex.
    fn is_hex(self, text: &str) -> bool {
        match self {
            Self::Sha256 => is_sha256_hex(text),
            Self::Md5 => {
                text.len() == 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            }
        }
    }
}

/// What this connection learned about running the hash command.
#[derive(Default)]
pub(super) struct DigestGate {
    unavailable: AtomicBool,
    failures: AtomicU32,
}

impl DigestGate {
    pub(super) fn closed(&self) -> bool {
        self.unavailable.load(Ordering::Relaxed)
    }

    pub(super) fn give_up(&self) {
        self.unavailable.store(true, Ordering::Relaxed);
    }

    pub(super) fn failed(&self) {
        if self.failures.fetch_add(1, Ordering::Relaxed) + 1 >= FAILURES_BEFORE_GIVING_UP {
            self.give_up();
        }
    }

    pub(super) fn worked(&self) {
        self.failures.store(0, Ordering::Relaxed);
    }
}

/// The command line of the SHA-256 script: the same for every path.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn command_for() -> String {
    command_line(Algorithm::Sha256)
}

/// The fixed command line of `algorithm`: `sh -c '<script>'`.
pub(super) fn command_line(algorithm: Algorithm) -> String {
    format!("sh -c '{}'", algorithm.script())
}

/// What goes to the command's standard input for `path`: `P`, the path,
/// nothing after it (the caller sends EOF). `None`: the path cannot be sent
/// (empty, or a NUL, which no shell variable can hold).
pub(super) fn stdin_for(path: &str) -> Option<Vec<u8>> {
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len() + 1);
    bytes.push(STDIN_PREFIX);
    bytes.extend_from_slice(path.as_bytes());
    Some(bytes)
}

/// How long to wait for the hash of `size` bytes.
pub(super) fn wait_for(size: u64) -> Duration {
    BASE_WAIT + Duration::from_secs(size / SLOWEST_HASHING)
}

/// What the finished command means.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Reply {
    /// The digest and the size the script read.
    Digest { hex: String, size: u64 },
    /// The script saw no regular file (or a link) at the path.
    NotRegular,
    /// This connection will never answer a hash command: the text did not
    /// run as the script (no marker), or the host has no tool.
    Unavailable,
    /// This time it did not work; commands may still work.
    Failed(String),
}

/// Reads the command's result: the script's marker line decides; without
/// it the shell did not run the script.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn interpret(capture: &CapturedExec) -> Reply {
    interpret_for(capture, Algorithm::Sha256)
}

/// `interpret` for the digest of `algorithm`.
pub(super) fn interpret_for(capture: &CapturedExec, algorithm: Algorithm) -> Reply {
    if capture.exit_signal.is_some() {
        return Reply::Failed("the hash command was ended by a signal".into());
    }
    let text = String::from_utf8_lossy(&capture.stdout);
    let answer = text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some(MARKER)).then(|| words.collect::<Vec<_>>())
    });
    let Some(words) = answer else {
        return match capture.exit_status {
            // A channel that closed without a status says nothing yet.
            None if !capture.stdout_truncated => {
                Reply::Failed("the hash command gave no exit status".into())
            }
            // Something else ran: cmd.exe, PowerShell, a forced command, a
            // shell that is no POSIX sh.
            _ => Reply::Unavailable,
        };
    };
    if capture.exit_status != Some(0) {
        return Reply::Failed(format!(
            "the hash script ended with status {:?}: {}",
            capture.exit_status,
            String::from_utf8_lossy(&capture.stderr).trim()
        ));
    }
    match words.as_slice() {
        ["ok", size, digest, ..] => {
            let hex = digest.trim_start_matches('\\').to_ascii_lowercase();
            match size.parse::<u64>() {
                Ok(size) if algorithm.is_hex(&hex) => Reply::Digest { hex, size },
                // The tool printed something else than a digest.
                _ => Reply::Unavailable,
            }
        }
        ["none"] => Reply::NotRegular,
        ["notool"] => Reply::Unavailable,
        ["fail"] => Reply::Failed(format!(
            "the hash tool failed: {}",
            String::from_utf8_lossy(&capture.stderr).trim()
        )),
        _ => Reply::Unavailable,
    }
}

/// The whole call, with the world behind two closures: `stat` (SFTP
/// attributes of the path) and `run` (the fixed command line, the bytes for
/// its standard input, how long to wait; the connection-level result of the
/// exec). `Ok(None)`: commands are not an option here and the caller moves
/// the content.
pub(super) fn digest_by(
    gate: &DigestGate,
    path: &str,
    stat: impl FnMut() -> VfsResult<VfsMeta>,
    run: impl FnOnce(&str, &[u8], Duration) -> io::Result<Option<CapturedExec>>,
) -> VfsResult<Option<ContentDigest>> {
    Ok(hash_by(gate, Algorithm::Sha256, path, stat, run)?
        .map(|(sha256, size)| ContentDigest { sha256, size }))
}

/// `digest_by` for any `algorithm`: the lowercase hex digest and the size the
/// SFTP attributes showed.
pub(super) fn hash_by(
    gate: &DigestGate,
    algorithm: Algorithm,
    path: &str,
    mut stat: impl FnMut() -> VfsResult<VfsMeta>,
    run: impl FnOnce(&str, &[u8], Duration) -> io::Result<Option<CapturedExec>>,
) -> VfsResult<Option<(String, u64)>> {
    if gate.closed() {
        return Ok(None);
    }
    let Some(stdin) = stdin_for(path) else {
        return Ok(None);
    };
    let before = stat()?;
    if before.is_dir || before.is_symlink || before.special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no regular file: {path}"),
        ));
    }
    let captured = match run(&command_line(algorithm), &stdin, wait_for(before.size)) {
        Ok(Some(captured)) => captured,
        // The server had no channel to spare right now.
        Ok(None) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            // The exec request itself was rejected (no shell for this
            // account, a subsystem-only login).
            gate.give_up();
            return Ok(None);
        }
        // Too slow: the content would arrive sooner than this hash.
        Err(error) if error.kind() == io::ErrorKind::TimedOut => {
            gate.failed();
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    match interpret_for(&captured, algorithm) {
        Reply::Digest { hex, size } => {
            let after = stat()?;
            if moved(&before, &after) {
                return Err(changed());
            }
            if size != before.size {
                // Same attributes over SFTP, another size for the shell: the
                // shell sees another file under this path.
                gate.give_up();
                return Ok(None);
            }
            gate.worked();
            Ok(Some((hex, before.size)))
        }
        Reply::NotRegular => {
            // Changed only when SFTP shows it; otherwise the shell's view
            // of the path is not SFTP's.
            let after = stat()?;
            if moved(&before, &after) {
                return Err(changed());
            }
            gate.give_up();
            Ok(None)
        }
        Reply::Unavailable => {
            gate.give_up();
            Ok(None)
        }
        Reply::Failed(_) => {
            gate.failed();
            Ok(None)
        }
    }
}

/// The SFTP attributes prove a change: another kind, size or time.
fn moved(before: &VfsMeta, after: &VfsMeta) -> bool {
    after.is_dir
        || after.is_symlink
        || after.special
        || after.size != before.size
        || after.mtime_ms != before.mtime_ms
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "file changed while its content digest was computed",
    )
}

impl SftpBackend {
    /// `BackendExtensions::content_digest` for this connection.
    pub(super) fn digest_by_command(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<ContentDigest>> {
        use crate::vfs::Backend;
        let gate: &Arc<DigestGate> = &self.digest_gate;
        digest_by(
            gate,
            path,
            || self.stat(path),
            |command, stdin, wait| super::digest_exec::run(self, command, stdin, wait, cancel),
        )
    }

    /// `BackendExtensions::content_md5` for this connection.
    pub(super) fn md5_by_command(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<String>> {
        use crate::vfs::Backend;
        let gate: &Arc<DigestGate> = &self.md5_gate;
        Ok(hash_by(
            gate,
            Algorithm::Md5,
            path,
            || self.stat(path),
            |command, stdin, wait| super::digest_exec::run(self, command, stdin, wait, cancel),
        )?
        .map(|(md5, _)| md5))
    }
}

#[cfg(test)]
#[path = "digest_command_tests.rs"]
mod tests;
