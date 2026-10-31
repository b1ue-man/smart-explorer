//! `content_digest` of an FTP server that hashes files itself: the SHA-256
//! comes back as one control-channel reply, no data connection is opened and
//! no content moves. Two commands are known:
//! - `HASH` (draft-bryan-ftpext-hash-02; FileZilla Server, ProFTPD, CompleteFTP
//!   and others) when FEAT lists `HASH` with `SHA-256`: `OPTS HASH SHA-256`
//!   selects the algorithm for this control connection, `HASH <path>` answers
//!   `213 SHA-256 <start>-<end> <hex> <path>`;
//! - `XSHA256 <path>` (ProFTPD mod_digest and servers that copied it) when FEAT
//!   lists `XSHA256`: the reply carries the digest as one 64-digit token.
//!
//! A server without either command, a refusal (500, 501, 502, 504, 550, 556 or
//! any other reply that is not the success reply), an algorithm other than the
//! one asked for or a hashed range that is not the whole file answer
//! `Ok(None)`: the caller moves the content. The size and the time of the
//! file must be the same before and after the command. The control read timeout
//! is raised to the time a hashing host may need (`wait_for`) and restored
//! afterwards; a cancel is looked at before and after the command, a running
//! command cannot be interrupted on the control channel (RFC 959 offers `ABOR`
//! for data transfers only), so it is bounded by that wait.
use super::connection::FTP_IO_TIMEOUT;
use super::core_impl::FtpBackend;
use super::errors::{command_path, map};
use super::metadata::Features;
use crate::vfs::{is_sha256_hex, Backend, ContentDigest, VfsResult};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use suppaftp::{FtpError, RustlsFtpStream, Status};

/// A host that hashes slower than this is not worth waiting for.
const SLOWEST_HASHING: u64 = 8 * 1024 * 1024;

/// How the server hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    Hash,
    XSha256,
}

impl Method {
    /// `HASH` before `XSHA256`; `None` when FEAT offers neither.
    pub(super) fn of(features: Features) -> Option<Self> {
        if features.hash_sha256 {
            Some(Self::Hash)
        } else if features.xsha256 {
            Some(Self::XSha256)
        } else {
            None
        }
    }
}

/// Time the control channel waits for the reply of a hash command.
pub(super) fn wait_for(size: u64) -> Duration {
    FTP_IO_TIMEOUT + Duration::from_secs(size / SLOWEST_HASHING)
}

/// The digest of `reply` to `HASH`, when it is the SHA-256 of exactly the
/// `size` bytes of the file. The draft's examples show an inclusive end
/// (`0-255` for 256 bytes); servers that name the size itself are accepted too.
pub(super) fn parse_hash_reply(reply: &str, size: u64) -> Option<String> {
    let mut words = reply.split_whitespace();
    if words.next()? != "213" || !words.next()?.eq_ignore_ascii_case("SHA-256") {
        return None;
    }
    let (start, end) = words.next()?.split_once('-')?;
    let (start, end): (u64, u64) = (start.parse().ok()?, end.parse().ok()?);
    if start != 0 || !(end == size || end.checked_add(1) == Some(size)) {
        return None;
    }
    let hex = words.next()?.to_ascii_lowercase();
    is_sha256_hex(&hex).then_some(hex)
}

/// The digest of `reply` to `XSHA256`: the one 64-digit token after the code.
pub(super) fn parse_bare_reply(reply: &str) -> Option<String> {
    let mut words = reply.split_whitespace();
    if !matches!(words.next()?, "213" | "250") {
        return None;
    }
    let mut tokens = words
        .map(str::to_ascii_lowercase)
        .filter(|word| is_sha256_hex(word));
    let hex = tokens.next()?;
    tokens.next().is_none().then_some(hex)
}

fn ask(
    stream: &mut RustlsFtpStream,
    method: Method,
    path: &str,
    size: u64,
) -> io::Result<Option<String>> {
    let reply = match method {
        Method::Hash => {
            match stream.custom_command("OPTS HASH SHA-256", &[Status::CommandOk]) {
                Ok(_) => {}
                Err(FtpError::UnexpectedResponse(_)) => return Ok(None),
                Err(error) => return Err(map(error)),
            }
            stream.custom_command(format!("HASH {path}"), &[Status::File])
        }
        Method::XSha256 => stream.custom_command(
            format!("XSHA256 {path}"),
            &[Status::File, Status::RequestedFileActionOk],
        ),
    };
    let response = match reply {
        Ok(response) => response,
        // The server answered, so the connection is in step; it just will
        // not hash this file.
        Err(FtpError::UnexpectedResponse(_)) => return Ok(None),
        Err(error) => return Err(map(error)),
    };
    let text = String::from_utf8_lossy(&response.body);
    Ok(match method {
        Method::Hash => parse_hash_reply(&text, size),
        Method::XSha256 => parse_bare_reply(&text),
    })
}

/// `ask` with the read timeout of a hashing host, restored afterwards.
fn ask_waiting(
    stream: &mut RustlsFtpStream,
    method: Method,
    path: &str,
    size: u64,
) -> io::Result<Option<String>> {
    stream.get_ref().set_read_timeout(Some(wait_for(size)))?;
    let answer = ask(stream, method, path, size);
    let restored = stream.get_ref().set_read_timeout(Some(FTP_IO_TIMEOUT));
    let answer = answer?;
    restored?;
    Ok(answer)
}

impl FtpBackend {
    /// `BackendExtensions::content_digest` for this server.
    pub(super) fn digest_by_command(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<ContentDigest>> {
        command_path(path)?;
        check_cancel(cancel)?;
        let before = self.stat(path)?;
        if before.is_dir || before.is_symlink || before.special {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no regular file: {path}"),
            ));
        }
        // A transfer connection when one is free, so a long hash does not
        // hold up browsing; else the browsing connection.
        let lease = self.pool.lease();
        let connection = match &lease {
            Ok(lease) => lease.connection(),
            Err(_) => self.pool.primary(),
        };
        let Some(method) = Method::of(connection.features()?) else {
            return Ok(None);
        };
        let hex = connection
            .with_stream_mutation(|stream| ask_waiting(stream, method, path, before.size))?;
        let Some(sha256) = hex else {
            return Ok(None);
        };
        check_cancel(cancel)?;
        let after = self.stat(path)?;
        if after.size != before.size
            || after.mtime_ms != before.mtime_ms
            || after.is_dir
            || after.is_symlink
            || after.special
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file changed while its content digest was computed",
            ));
        }
        Ok(Some(ContentDigest {
            sha256,
            size: before.size,
        }))
    }
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "content digest canceled",
        ));
    }
    Ok(())
}
