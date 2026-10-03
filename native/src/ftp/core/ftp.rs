//! FTP / FTPS backend (`suppaftp`, blocking) implementing `vfs::Backend`.
//!
//! One `RustlsFtpStream` type carries both plain FTP (`ftp://`) and explicit
//! FTPS (`ftps://` — AUTH TLS after connect). TLS is rustls backed by **ring**
//! (no native-tls / schannel FFI on GNU; see docs/GOTCHAS.md) with bundled
//! webpki-roots. Browsing and metadata run on one control connection
//! (`parallelism() == 1`); every concurrent transfer borrows a control
//! connection of its own from the pool (pool.rs), whose size the server's
//! connection limit bounds.
//!
//! Listings are parsed by suppaftp's `list::File` (posix / dos / mlsx). RETR
//! streams directly from the data connection (after REST to resume). Uploads
//! spool to disk and issue a streaming STOR only at the caller's explicit
//! `flush` commit boundary; a copy stage of known length streams its STOR.

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::connection::{connect_stream, parse_ftp_url, refusal_code};
use super::io_adapters::{FtpConnection, FtpReconnect};
use super::pool::{FtpPool, Leased};
use super::writer::FtpWriter;
use suppaftp::FtpError;

#[cfg(test)]
use super::connection::FtpUrl;
#[cfg(test)]
use std::time::Duration;

fn io_err(error: FtpError) -> io::Error {
    super::errors::map(error)
}

fn systime_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

pub(super) fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

pub(super) fn parent_dir(path: &str) -> String {
    let t = path.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => t[..i].to_string(),
    }
}

pub(super) fn dir_meta(name: String) -> VfsMeta {
    VfsMeta {
        name,
        is_dir: true,
        is_symlink: false,
        special: false,
        size: 0,
        mtime_ms: 0,
        btime_ms: 0,
        hidden: false,
        system: false,
        id: None,
        content_md5: None,
    }
}

pub(super) fn parse_list_line(line: &str) -> VfsResult<VfsMeta> {
    let meta = parse_list_line_unchecked(line)?;
    crate::vfs::validate_child_name(&meta.name)?;
    Ok(meta)
}

pub(super) fn parse_list_line_unchecked(line: &str) -> VfsResult<VfsMeta> {
    let file = line.parse::<suppaftp::list::File>().map_err(|error| {
        let preview: String = line.chars().take(160).collect();
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("FTP LIST row could not be parsed ({error}): {preview:?}"),
        )
    })?;
    let name = file.name().to_string();
    Ok(VfsMeta {
        is_dir: file.is_directory(),
        is_symlink: file.is_symlink(),
        special: line
            .as_bytes()
            .first()
            .is_some_and(|kind| matches!(kind, b'b' | b'c' | b'p' | b's')),
        size: file.size() as u64,
        mtime_ms: systime_ms(file.modified()),
        btime_ms: 0,
        hidden: name.starts_with('.'),
        system: false,
        name,
        id: None,
        content_md5: None,
    })
}

// ── URL ──────────────────────────────────────────────────────────────────────

// ── backend ──────────────────────────────────────────────────────────────────

pub struct FtpBackend {
    pub(super) pool: Arc<FtpPool>,
    root: String,
    /// `ftp(s)://user@host:port/root` for UI display (connect-UI step).
    #[allow(dead_code)]
    url: String,
}

/// Connect from an `ftp://` / `ftps://` URL. Plain FTP allows anonymous login,
/// so (unlike SFTP) a bare URL connects without a credential dialog.
pub fn backend_from_url(url: &str) -> io::Result<FtpBackend> {
    let u = parse_ftp_url(url)?;
    let ftp = connect_stream(&u)?;
    let url = format!(
        "{}://{}@{}:{}{}",
        if u.secure { "ftps" } else { "ftp" },
        u.user,
        u.host,
        u.port,
        u.root
    );
    let reconnect_config = u.clone();
    let reconnect: FtpReconnect = Arc::new(move || connect_stream(&reconnect_config));
    let primary = FtpConnection::new(ftp, reconnect.clone())?;
    Ok(FtpBackend {
        pool: FtpPool::new(primary, reconnect),
        root: u.root,
        url,
    })
}

/// A pooled connection that had to log in again and was turned away: the
/// server's connection limit, so the transfer backs off (plan K13).
fn transfer_err(error: io::Error) -> io::Error {
    match refusal_code(&error) {
        Some(_) => crate::vfs::congestion_error(error.to_string(), None),
        None => error,
    }
}

impl FtpBackend {
    /// Publishes a stage on a pooled connection when one is free, else on
    /// the browsing connection, so it never fails for want of a connection.
    fn publish(&self, staged: &str, destination: &str, replace: bool) -> VfsResult<()> {
        match self.pool.lease() {
            Ok(lease) => super::staging::publish(lease.connection(), staged, destination, replace),
            Err(_) => super::staging::publish(self.pool.primary(), staged, destination, replace),
        }
    }

    /// One MKD. An existing real folder is fine unless `exclusive`: the
    /// server's MKD refuses an existing name atomically, the entry is only
    /// looked at afterwards to tell `AlreadyExists` from other refusals.
    fn make_dir(&self, path: &str, exclusive: bool) -> VfsResult<()> {
        super::errors::command_path(path)?;
        let made = self
            .pool
            .primary()
            .with_stream_mutation(|stream| match stream.mkdir(path) {
                Ok(()) => Ok(Ok(())),
                // The server answered, so the connection is in step.
                Err(error @ FtpError::UnexpectedResponse(_)) => Ok(Err(io_err(error))),
                Err(error) => Err(io_err(error)),
            })?;
        let Err(refusal) = made else {
            return Ok(());
        };
        match self.stat(path) {
            Ok(meta) if meta.is_dir && !meta.is_symlink && !exclusive => Ok(()),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{path} existiert bereits"),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(refusal),
            Err(error) => Err(error),
        }
    }
}

impl Backend for FtpBackend {
    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        Some(self)
    }
    fn scheme(&self) -> Scheme {
        Scheme::Ftp
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }
    fn state_identity(&self) -> String {
        self.url.clone()
    }
    fn namespace_identity(&self) -> String {
        self.url
            .strip_suffix(&self.root)
            .unwrap_or(&self.url)
            .to_string()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        // Browsing is what follows a burst of transfers: close their idle
        // connections here instead of keeping them alive with NOOPs.
        self.pool.retire_idle();
        let listing = super::metadata::list_browse(self.pool.primary(), path)?;
        if !listing.omitted.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "FTP listing has protected omissions; use list_dir_tolerant",
            ));
        }
        Ok(listing.entries)
    }

    fn list_dir_for_sync(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let listing = super::metadata::list(self.pool.primary(), path)?;
        if !listing.omitted.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "FTP sync listing has protected omissions; use list_dir_tolerant",
            ));
        }
        Ok(listing.entries)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        super::metadata::stat(self.pool.primary(), path)
    }

    /// RETR on a pool connection of its own, so browsing and other
    /// transfers go on meanwhile.
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let lease = self.pool.lease()?;
        let reader = lease.connection().open_reader(path).map_err(transfer_err)?;
        Ok(Box::new(Leased::new(reader, lease)))
    }

    /// REST + RETR; `None` when the server has no REST.
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let _ = id;
        let lease = self.pool.lease()?;
        let reader = lease
            .connection()
            .open_reader_at(path, offset)
            .map_err(transfer_err)?;
        Ok(reader.map(|reader| Box::new(Leased::new(reader, lease)) as Box<dyn Read + Send>))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(FtpWriter::new(
            self.pool.primary().clone(),
            path.to_string(),
        )?))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        super::errors::command_path(src)?;
        super::errors::command_path(dst)?;
        self.pool
            .primary()
            .with_stream_mutation(|stream| stream.rename(src, dst).map_err(io_err))
    }

    // Staged writes: see staging.rs (no exclusive create or no-replace rename in FTP).
    fn open_write_copy_stage(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        super::staging::require_absent(self, path)?;
        self.open_write(path)
    }

    /// With the length known, STOR streams on a pool connection instead of
    /// spooling the whole file to a local temp file first; a writer that did
    /// not get exactly `size` bytes fails `flush`. The stage's name is checked
    /// on the same connection (staging.rs).
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        let lease = self.pool.lease().map_err(transfer_err)?;
        super::staging::require_stage_free(lease.connection(), path)?;
        let writer = lease
            .connection()
            .open_store(path, size)
            .map_err(transfer_err)?;
        Ok(Box::new(Leased::new(writer, lease)))
    }

    // `rename_no_replace` stays unsupported (trait contract); only publishing a
    // stage uses the absence check (FTP exception on `vfs::promote_staged_create`).
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.publish(staged, destination, false)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.publish(staged, destination, true)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        super::errors::command_path(path)?;
        self.pool
            .primary()
            .with_stream_mutation(|stream| stream.rm(path).map_err(io_err))
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        super::errors::command_path(path)?;
        self.pool
            .primary()
            .with_stream_mutation(|stream| stream.rmdir(path).map_err(io_err))
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        super::errors::command_path(path)?;
        let absolute = path.starts_with('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.pool.primary().with_stream_mutation(|stream| {
            let original = stream.pwd().map_err(io_err)?;
            let mut cur = String::new();
            for part in parts {
                if cur.is_empty() {
                    if absolute {
                        cur.push('/');
                    }
                } else {
                    cur.push('/');
                }
                cur.push_str(part);
                if let Err(mkdir_error) = stream.mkdir(&cur) {
                    stream.cwd(&cur).map_err(|verify_error| {
                        io::Error::other(format!(
                            "FTP mkdir failed for {cur}: {mkdir_error}; existing-directory verification failed: {verify_error}"
                        ))
                    })?;
                    stream.cwd(&original).map_err(io_err)?;
                }
            }
            Ok(())
        })
    }

    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.make_dir(path, false)
    }

    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.make_dir(path, true)
    }

    fn parallelism(&self) -> usize {
        1 // one browsing control connection
    }

    /// Every transfer holds a control connection; the pool learns how many
    /// the server accepts and keeps one for browsing (plan K24).
    fn transfer_ceiling(&self, path: &str) -> Option<usize> {
        let _ = path;
        self.pool
            .transfer_capacity()
            .map(|capacity| capacity.max(1))
    }

    fn flow_key(&self, path: &str) -> String {
        let _ = path;
        format!(
            "{}#{:p}",
            self.namespace_identity(),
            Arc::as_ptr(&self.pool)
        )
    }

    /// A download and an upload on the same server need two transfer
    /// connections at once; until the server has refused one, the pool can
    /// still grow.
    fn concurrent_read_write(&self) -> bool {
        self.pool
            .transfer_capacity()
            .is_none_or(|capacity| capacity >= 2)
    }
}

#[cfg(test)]
#[path = "ftp_tests.rs"]
mod tests;
