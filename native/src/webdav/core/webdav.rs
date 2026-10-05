//! WebDAV cloud backend implementing `vfs::Backend` over the project's verified
//! ring-rustls `ureq` (no extra TLS stack). Covers Nextcloud / ownCloud / any
//! WebDAV server with HTTP Basic auth. Directory listings use `PROPFIND`
//! (Depth 1) parsed with `roxmltree`; the rest is GET / PUT / DELETE / MKCOL /
//! MOVE / COPY. Blocking, so no runtime is needed.
//!
//! Demonstrates that "cloud" storage drops onto the SAME `Backend` interface —
//! S3 / OAuth providers (Google Drive, OneDrive, Dropbox) slot in the same way
//! (a new module + a Connect-dialog protocol); WebDAV is shipped first because
//! it needs only username/password, no per-provider OAuth app registration.

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use super::multistatus::{encode_path, parse_multistatus};
use super::status::overload_or_full;
use super::writer::WebdavWriter;

#[path = "transfer_ops.rs"]
mod transfer_ops;

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

pub(super) fn request_err(error: ureq::Error) -> io::Error {
    if let Some(mapped) = overload_or_full(&error) {
        return mapped;
    }
    let kind = match &error {
        ureq::Error::Status(404, _) => io::ErrorKind::NotFound,
        ureq::Error::Status(412, _) => io::ErrorKind::AlreadyExists,
        ureq::Error::Status(401 | 403, _) => io::ErrorKind::PermissionDenied,
        // Only a ranged GET gets it: the file is shorter than the resume point.
        ureq::Error::Status(416, _) => io::ErrorKind::InvalidData,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error.to_string())
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);
/// Idle connections kept per host by each pooled agent: ureq's overall
/// default (`max_idle_connections`, 100) instead of its per-host default of
/// one, so parallel transfers find their connection again instead of paying
/// TCP and TLS setup per request. The pool never keeps more connections than
/// were in use at once.
const IDLE_CONNECTIONS_PER_HOST: usize = 100;

fn transport_builder() -> ureq::AgentBuilder {
    let builder = ureq::AgentBuilder::new();
    #[cfg(test)]
    if let Some(path) = std::env::var_os("SE_SYNC_FIXTURE_CA_DER") {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let der = std::fs::read(path).expect("read owned sync fixture CA");
        roots
            .add(rustls::pki_types::CertificateDer::from(der))
            .expect("valid owned sync fixture CA");
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring supports default protocols")
        .with_root_certificates(roots)
        .with_no_client_auth();
        return builder.tls_config(Arc::new(config));
    }
    builder
}

pub struct WebdavConfig {
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub root: String,
}

#[derive(Clone)]
pub struct WebdavBackend {
    base: String,            // scheme://host:port
    pub(super) root: String, // forward-slash path
    pub(super) auth: String, // "Basic ..." (empty = none)
    /// Pooled agent for idempotent reads; ureq replaces stale pooled sockets.
    agent: ureq::Agent,
    /// Unpooled agent for DELETE and an empty PUT, which ureq would replay on
    /// a fresh connection after an ambiguous response loss on a recycled one.
    pub(super) mutation_agent: ureq::Agent,
    /// Pooled agent for the mutations ureq never replays: PUT with a body and
    /// MOVE, MKCOL, COPY (not in its idempotent list; gdrive-ureq-throughput.md
    /// §8). Saves TCP and TLS setup per upload, folder and rename.
    pub(super) write_agent: ureq::Agent,
    /// Display label, consumed by the connect-UI step.
    #[allow(dead_code)]
    url: String,
    identity: String,
    pub(super) hashes_observed: Arc<AtomicBool>,
    pub(super) stage_times: Arc<Mutex<HashMap<String, (i64, Option<String>)>>>,
}

impl WebdavBackend {
    pub fn connect(cfg: WebdavConfig) -> io::Result<WebdavBackend> {
        let scheme = if cfg.https { "https" } else { "http" };
        let host = cfg.host.trim();
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let base = format!("{scheme}://{host}:{}", cfg.port);
        let auth = if cfg.user.is_empty() {
            String::new()
        } else {
            format!(
                "Basic {}",
                STANDARD.encode(format!("{}:{}", cfg.user, cfg.password))
            )
        };
        let agent = transport_builder()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(IO_INACTIVITY_TIMEOUT)
            .timeout_write(IO_INACTIVITY_TIMEOUT)
            .max_idle_connections_per_host(IDLE_CONNECTIONS_PER_HOST)
            .build();
        let mutation_agent = transport_builder()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(IO_INACTIVITY_TIMEOUT)
            .timeout_write(IO_INACTIVITY_TIMEOUT)
            .redirects(0)
            .max_idle_connections(0)
            .build();
        let write_agent = transport_builder()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(IO_INACTIVITY_TIMEOUT)
            .timeout_write(IO_INACTIVITY_TIMEOUT)
            .redirects(0)
            .max_idle_connections_per_host(IDLE_CONNECTIONS_PER_HOST)
            .build();
        let root = if cfg.root.trim().is_empty() {
            "/".to_string()
        } else {
            cfg.root.to_string()
        };
        let identity = format!("webdav:{base}:user={}:root={root}", cfg.user);
        let be = WebdavBackend {
            url: format!("webdav {}{}", base, root),
            base,
            root: root.clone(),
            auth,
            agent,
            mutation_agent,
            write_agent,
            identity,
            hashes_observed: Arc::new(AtomicBool::new(false)),
            stage_times: Arc::new(Mutex::new(HashMap::new())),
        };
        // Validate credentials / reachability up front.
        be.propfind(&root, "0")?;
        Ok(be)
    }

    #[allow(dead_code)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub(super) fn url_for(&self, path: &str) -> String {
        format!("{}{}", self.base, encode_path(path))
    }

    pub(super) fn auth_req(&self, req: ureq::Request) -> ureq::Request {
        if self.auth.is_empty() {
            req
        } else {
            req.set("Authorization", &self.auth)
        }
    }

    pub(super) fn propfind(
        &self,
        path: &str,
        depth: &str,
    ) -> io::Result<super::listing_body::Body> {
        // Also request ownCloud/Nextcloud's checksums (free content hashes) so a
        // checksum-mode sync can compare without downloading. Plain WebDAV servers
        // ignore the oc:* prop.
        let body = r#"<?xml version="1.0" encoding="utf-8"?><propfind xmlns="DAV:" xmlns:oc="http://owncloud.org/ns"><prop><resourcetype/><getcontentlength/><getlastmodified/><getetag/><oc:checksums/></prop></propfind>"#;
        for attempt in 0..2 {
            let req = self
                .agent
                .request("PROPFIND", &self.url_for(path))
                .set("Depth", depth)
                .set("Content-Type", "application/xml");
            let req = self.auth_req(req);
            match req.send_string(body) {
                Ok(response) => match super::listing_body::read(response) {
                    Ok(body) => return Ok(body),
                    Err(error)
                        if attempt == 0
                            && matches!(
                                error.kind(),
                                io::ErrorKind::UnexpectedEof
                                    | io::ErrorKind::ConnectionReset
                                    | io::ErrorKind::ConnectionAborted
                                    | io::ErrorKind::BrokenPipe
                                    | io::ErrorKind::TimedOut
                            ) =>
                    {
                        continue
                    }
                    Err(error) => return Err(error),
                },
                Err(ureq::Error::Transport(_)) if attempt == 0 => continue,
                Err(error) => return Err(request_err(error)),
            }
        }
        Err(io::Error::other("WebDAV PROPFIND retry exhausted"))
    }

    fn get(&self, path: &str) -> io::Result<ureq::Response> {
        self.get_from(path, 0)
    }

    /// GET from byte `offset` (a `Range` request when it is not 0).
    fn get_from(&self, path: &str, offset: u64) -> io::Result<ureq::Response> {
        for attempt in 0..2 {
            let request = self.agent.get(&self.url_for(path));
            let request = if offset == 0 {
                request
            } else {
                request.set("Range", &format!("bytes={offset}-"))
            };
            match self.auth_req(request).call() {
                Ok(response) => return Ok(response),
                Err(ureq::Error::Transport(_)) if attempt == 0 => continue,
                Err(error) => return Err(request_err(error)),
            }
        }
        Err(io::Error::other("WebDAV GET retry exhausted"))
    }

    pub(super) fn mutation(
        &self,
        request: ureq::Request,
        operation: &str,
    ) -> io::Result<ureq::Response> {
        let response = self.auth_req(request).call().map_err(request_err)?;
        let status = response.status();
        if !(200..300).contains(&status) || status == 207 {
            return Err(io::Error::other(format!(
                "WebDAV {operation} returned unexpected HTTP status {status}"
            )));
        }
        Ok(response)
    }
}

impl Backend for WebdavBackend {
    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        Some(self)
    }
    fn scheme(&self) -> Scheme {
        Scheme::Webdav
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }
    fn state_identity(&self) -> String {
        self.identity.clone()
    }
    fn namespace_identity(&self) -> String {
        self.identity
            .strip_suffix(&format!(":root={}", self.root))
            .unwrap_or(&self.identity)
            .to_string()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let xml = self.propfind(path, "1")?;
        let entries = parse_multistatus(&xml, path)?;
        if entries.iter().any(|entry| entry.content_md5.is_some()) {
            self.hashes_observed.store(true, Ordering::Relaxed);
        }
        Ok(entries)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let xml = self.propfind(path, "0")?;
        let (meta, _) = super::metadata::parse(&xml, path)?;
        if meta.content_md5.is_some() {
            self.hashes_observed.store(true, Ordering::Relaxed);
        }
        Ok(meta)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        // Retrying is safe only until a response body is handed to the caller:
        // no file bytes have been observed and GET itself is idempotent. A
        // failure while the returned reader is in use remains visible instead
        // of silently restarting a possibly changed object.
        let resp = self.get(path)?;
        Ok(Box::new(resp.into_reader()))
    }

    /// A ranged GET resumes; `None` when the server sends the whole file.
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let _ = id;
        self.read_from(path, offset)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(WebdavWriter::new(
            self.write_agent.clone(),
            self.mutation_agent.clone(),
            self.url_for(path),
            self.auth.clone(),
        )?))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(WebdavWriter::new_exclusive(
            self.write_agent.clone(),
            self.mutation_agent.clone(),
            self.url_for(path),
            self.auth.clone(),
        )?))
    }

    /// With the length known, the PUT streams while it is written instead of
    /// spooling to a temp file first (stream_put.rs).
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer(path, size)
    }

    /// COPY with `Overwrite: F` into the stage: the bytes never leave the
    /// server. `None` when the server has no COPY (the engine streams).
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<u64>> {
        let _ = size;
        self.copy_to_stage(src, stage)
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let staged = crate::vfs::unique_staging_path(self, dst, "copy")?;
        let result = (|| {
            self.mutation(
                self.write_agent
                    .request("COPY", &self.url_for(src))
                    .set("Destination", &self.url_for(&staged))
                    .set("Overwrite", "F"),
                "COPY",
            )?;
            let size = self.stat(&staged)?.size;
            crate::vfs::promote_staged_replace(self, &staged, dst)?;
            Ok(size)
        })();
        if result.is_err() {
            let _ = self.remove_file(&staged);
        }
        result
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.move_stage_time(src, dst, true)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.move_stage_time(src, dst, false)
    }

    /// Replaces an existing file with one `MOVE` and `Overwrite: T` (RFC 4918):
    /// the server swaps the files in a single request, so no client-side
    /// remove-then-rename can be left half done. Not an old-or-new atomic
    /// guarantee, which is why mounts keep requiring `rename_overwrites`.
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        crate::vfs::promote_staged_with(self, staged, destination, |staged, destination| {
            self.rename(staged, destination)
        })
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.mutation(
            self.mutation_agent.request("DELETE", &self.url_for(path)),
            "DELETE",
        )?;
        if let Ok(mut times) = self.stage_times.lock() {
            times.remove(path);
        }
        Ok(())
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.remove_file(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.mkdir_below_root(path)
    }

    /// One MKCOL; an existing collection is fine.
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.create_collection(path, false)
    }

    /// One MKCOL on a free name: 405 means taken (RFC 4918 §9.3.1).
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.create_collection(path, true)
    }

    /// Stages are created with `If-None-Match: *` or `COPY Overwrite: F`,
    /// so the name is this client's until it is published.
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.discard_stage(stage)
    }

    fn parallelism(&self) -> usize {
        2 // HTTP keep-alive; a couple of concurrent requests are fine
    }

    fn rename_overwrites(&self) -> bool {
        // RFC 4918 MOVE Overwrite:T deletes the destination before moving the
        // source; it is not an old-or-new atomic replacement guarantee.
        false
    }

    fn staged_write_capabilities(&self, _root: &str) -> crate::vfs::StagedWriteCapabilities {
        crate::vfs::StagedWriteCapabilities {
            create: true,
            replace: false,
            namespace_replace: false,
        }
    }

    fn provides_content_hash(&self) -> bool {
        // Nextcloud/ownCloud expose an MD5 via the `oc:checksums` PROPFIND prop
        // (parsed into `content_md5`) — a free content hash, no download. Servers
        // that don't send one leave `content_md5` None, so those files simply
        // fall back to the size+mtime compare (graceful per-file degradation).
        self.hashes_observed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod connection_tests;

#[cfg(test)]
#[path = "promote_tests.rs"]
mod promote_tests;
