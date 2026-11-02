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
//! `content_md5` (the sync signature, `vfs::content_md5`) asks the same way for
//! the MD5: `HASH` after `OPTS HASH MD5` when FEAT lists `MD5` in `HASH`
//! (reply `213 MD5 <start>-<end> <hex> <path>`), else `XMD5 <path>` when FEAT
//! lists `XMD5` (reply `250`, `251` or `213` with one 32-digit token).
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
use crate::vfs::{Backend, ContentDigest, VfsResult};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use suppaftp::{FtpError, RustlsFtpStream, Status};

/// A host that hashes slower than this is not worth waiting for.
const SLOWEST_HASHING: u64 = 8 * 1024 * 1024;

/// What the server is asked to form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Algorithm {
    Sha256,
    Md5,
}

impl Algorithm {
    /// The name `OPTS HASH` and the `HASH` reply use.
    fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "SHA-256",
            Self::Md5 => "MD5",
        }
    }

    /// The vendor command that answers with the bare digest.
    fn bare_command(self) -> &'static str {
        match self {
            Self::Sha256 => "XSHA256",
            Self::Md5 => "XMD5",
        }
    }

    /// Whether `text` is a digest of this algorithm in lowercase hex.
    fn is_hex(self, text: &str) -> bool {
        let digits = match self {
            Self::Sha256 => 64,
            Self::Md5 => 32,
        };
        text.len() == digits && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }
}

/// How the server hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    Hash,
    /// `XSHA256` / `XMD5`.
    Bare,
}

impl Method {
    /// `HASH` before the vendor command; `None` when FEAT offers neither for
    /// `algorithm`.
    pub(super) fn of(features: Features, algorithm: Algorithm) -> Option<Self> {
        let (hash, bare) = match algorithm {
            Algorithm::Sha256 => (features.hash_sha256, features.xsha256),
            Algorithm::Md5 => (features.hash_md5, features.xmd5),
        };
        if hash {
            Some(Self::Hash)
        } else if bare {
            Some(Self::Bare)
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
#[cfg(any(test, feature = "test-support"))]
pub(super) fn parse_hash_reply(reply: &str, size: u64) -> Option<String> {
    parse_hash_reply_for(reply, size, Algorithm::Sha256)
}

/// `parse_hash_reply` for `algorithm`.
pub(super) fn parse_hash_reply_for(reply: &str, size: u64, algorithm: Algorithm) -> Option<String> {
    let mut words = reply.split_whitespace();
    if words.next()? != "213" || !words.next()?.eq_ignore_ascii_case(algorithm.name()) {
        return None;
    }
    let (start, end) = words.next()?.split_once('-')?;
    let (start, end): (u64, u64) = (start.parse().ok()?, end.parse().ok()?);
    if start != 0 || !(end == size || end.checked_add(1) == Some(size)) {
        return None;
    }
    let hex = words.next()?.to_ascii_lowercase();
    algorithm.is_hex(&hex).then_some(hex)
}

/// The digest of `reply` to `XSHA256`: the one 64-digit token after the code.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn parse_bare_reply(reply: &str) -> Option<String> {
    parse_bare_reply_for(reply, Algorithm::Sha256)
}

/// The digest of `reply` to `XSHA256` or `XMD5`: the one token of the
/// algorithm's length after the code (`251` is the code of `XMD5`).
pub(super) fn parse_bare_reply_for(reply: &str, algorithm: Algorithm) -> Option<String> {
    let mut words = reply.split_whitespace();
    if !matches!(words.next()?, "213" | "250" | "251") {
        return None;
    }
    let mut tokens = words
        .map(str::to_ascii_lowercase)
        .filter(|word| algorithm.is_hex(word));
    let hex = tokens.next()?;
    tokens.next().is_none().then_some(hex)
}

fn ask(
    stream: &mut RustlsFtpStream,
    method: Method,
    algorithm: Algorithm,
    path: &str,
    size: u64,
) -> io::Result<Option<String>> {
    let reply = match method {
        Method::Hash => {
            let select = format!("OPTS HASH {}", algorithm.name());
            match stream.custom_command(select, &[Status::CommandOk]) {
                Ok(_) => {}
                Err(FtpError::UnexpectedResponse(_)) => return Ok(None),
                Err(error) => return Err(map(error)),
            }
            stream.custom_command(format!("HASH {path}"), &[Status::File])
        }
        // 251 has no name in `Status`; it arrives as `Unknown` and the reply
        // parser looks at the code itself.
        Method::Bare => stream.custom_command(
            format!("{} {path}", algorithm.bare_command()),
            &[Status::File, Status::RequestedFileActionOk, Status::Unknown],
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
        Method::Hash => parse_hash_reply_for(&text, size, algorithm),
        Method::Bare => parse_bare_reply_for(&text, algorithm),
    })
}

/// `ask` with the read timeout of a hashing host, restored afterwards.
fn ask_waiting(
    stream: &mut RustlsFtpStream,
    method: Method,
    algorithm: Algorithm,
    path: &str,
    size: u64,
) -> io::Result<Option<String>> {
    stream.get_ref().set_read_timeout(Some(wait_for(size)))?;
    let answer = ask(stream, method, algorithm, path, size);
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
        Ok(self
            .hashed_by_server(path, cancel, Algorithm::Sha256)?
            .map(|(sha256, size)| ContentDigest { sha256, size }))
    }

    /// `BackendExtensions::content_md5` for this server.
    pub(super) fn md5_by_command(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<String>> {
        Ok(self
            .hashed_by_server(path, cancel, Algorithm::Md5)?
            .map(|(md5, _)| md5))
    }

    /// The lowercase hex digest of `algorithm` and the size of the file the
    /// server hashed; `Ok(None)` when it offers no way or refuses.
    fn hashed_by_server(
        &self,
        path: &str,
        cancel: &AtomicBool,
        algorithm: Algorithm,
    ) -> VfsResult<Option<(String, u64)>> {
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
        let Some(method) = Method::of(connection.features()?, algorithm) else {
            return Ok(None);
        };
        let hex = connection.with_stream_mutation(|stream| {
            ask_waiting(stream, method, algorithm, path, before.size)
        })?;
        let Some(hex) = hex else {
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
        Ok(Some((hex, before.size)))
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
