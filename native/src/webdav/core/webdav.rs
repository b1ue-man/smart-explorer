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
use std::io::{self, Read, Write};
use std::time::Duration;

use super::multistatus::{
    basename, encode_path, parse_http_date_ms, parse_multistatus, validate_propfind_response,
};
use super::writer::WebdavWriter;

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

fn request_err(error: ureq::Error) -> io::Error {
    let kind = match &error {
        ureq::Error::Status(404, _) => io::ErrorKind::NotFound,
        ureq::Error::Status(412, _) => io::ErrorKind::AlreadyExists,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error.to_string())
}

const TIMEOUT: Duration = Duration::from_secs(30);

pub struct WebdavConfig {
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub root: String,
}

pub struct WebdavBackend {
    base: String, // scheme://host:port
    root: String, // forward-slash path
    auth: String, // "Basic ..." (empty = none)
    agent: ureq::Agent,
    /// Display label, consumed by the connect-UI step.
    #[allow(dead_code)]
    url: String,
    identity: String,
}

impl WebdavBackend {
    pub fn connect(cfg: WebdavConfig) -> io::Result<WebdavBackend> {
        let scheme = if cfg.https { "https" } else { "http" };
        let base = format!("{}://{}:{}", scheme, cfg.host.trim(), cfg.port);
        let auth = if cfg.user.is_empty() {
            String::new()
        } else {
            format!(
                "Basic {}",
                STANDARD.encode(format!("{}:{}", cfg.user, cfg.password))
            )
        };
        let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
        let root = if cfg.root.trim().is_empty() {
            "/".to_string()
        } else {
            cfg.root.trim().to_string()
        };
        let identity = format!("webdav:{base}:user={}:root={root}", cfg.user);
        let be = WebdavBackend {
            url: format!("webdav {}{}", base, root),
            base,
            root: root.clone(),
            auth,
            agent,
            identity,
        };
        // Validate credentials / reachability up front.
        be.propfind(&root, "0")?;
        Ok(be)
    }

    #[allow(dead_code)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    fn url_for(&self, path: &str) -> String {
        format!("{}{}", self.base, encode_path(path))
    }

    fn auth_req(&self, req: ureq::Request) -> ureq::Request {
        if self.auth.is_empty() {
            req
        } else {
            req.set("Authorization", &self.auth)
        }
    }

    fn propfind(&self, path: &str, depth: &str) -> io::Result<String> {
        // Also request ownCloud/Nextcloud's checksums (free content hashes) so a
        // checksum-mode sync can compare without downloading. Plain WebDAV servers
        // ignore the oc:* prop.
        let body = r#"<?xml version="1.0" encoding="utf-8"?><propfind xmlns="DAV:" xmlns:oc="http://owncloud.org/ns"><prop><resourcetype/><getcontentlength/><getlastmodified/><oc:checksums/></prop></propfind>"#;
        let req = self
            .agent
            .request("PROPFIND", &self.url_for(path))
            .set("Depth", depth)
            .set("Content-Type", "application/xml");
        let req = self.auth_req(req);
        // 207 Multi-Status is a 2xx, so ureq returns Ok.
        req.send_string(body)
            .map_err(request_err)?
            .into_string()
            .map_err(io_err)
    }

    fn mutation(&self, request: ureq::Request, operation: &str) -> io::Result<ureq::Response> {
        let response = self.auth_req(request).call().map_err(request_err)?;
        if response.status() == 207 {
            return Err(io::Error::other(format!(
                "WebDAV {operation} returned a partial Multi-Status response"
            )));
        }
        Ok(response)
    }
}

impl Backend for WebdavBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Webdav
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }
    fn state_identity(&self) -> String {
        self.identity.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let xml = self.propfind(path, "1")?;
        parse_multistatus(&xml, path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let xml = self.propfind(path, "0")?;
        // Depth 0 returns the resource itself; parse without dropping self.
        let doc = roxmltree::Document::parse(&xml).map_err(io_err)?;
        let resp = doc
            .descendants()
            .find(|n| n.tag_name().name() == "response")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "kein PROPFIND-Response"))?;
        validate_propfind_response(resp)?;
        let is_dir = resp
            .descendants()
            .any(|n| n.tag_name().name() == "collection");
        let size = resp
            .descendants()
            .find(|n| n.tag_name().name() == "getcontentlength")
            .and_then(|n| n.text())
            .and_then(|t| t.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let mtime_ms = resp
            .descendants()
            .find(|n| n.tag_name().name() == "getlastmodified")
            .and_then(|n| n.text())
            .and_then(parse_http_date_ms)
            .unwrap_or(0);
        let name = basename(path);
        Ok(VfsMeta {
            is_dir,
            is_symlink: false,
            size: if is_dir { 0 } else { size },
            mtime_ms,
            btime_ms: 0,
            hidden: name.starts_with('.'),
            system: false,
            name,
            id: None,
            content_md5: None,
        })
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let resp = self
            .auth_req(self.agent.get(&self.url_for(path)))
            .call()
            .map_err(request_err)?;
        Ok(Box::new(resp.into_reader()))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(WebdavWriter::new(
            self.agent.clone(),
            self.url_for(path),
            self.auth.clone(),
        )?))
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let staged = crate::vfs::unique_staging_path(self, dst, "copy")?;
        let result = (|| {
            self.mutation(
                self.agent
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
        self.mutation(
            self.agent
                .request("MOVE", &self.url_for(src))
                .set("Destination", &self.url_for(dst))
                .set("Overwrite", "T"),
            "MOVE",
        )?;
        Ok(())
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.mutation(
            self.agent
                .request("MOVE", &self.url_for(src))
                .set("Destination", &self.url_for(dst))
                .set("Overwrite", "F"),
            "MOVE",
        )?;
        Ok(())
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.mutation(self.agent.request("DELETE", &self.url_for(path)), "DELETE")?;
        Ok(())
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.remove_file(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let absolute = path.starts_with('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
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
            match self
                .auth_req(self.agent.request("MKCOL", &self.url_for(&cur)))
                .call()
            {
                Ok(response) if response.status() != 207 => {}
                Ok(_) => {
                    return Err(io::Error::other(
                        "WebDAV MKCOL returned a partial Multi-Status response",
                    ));
                }
                Err(ureq::Error::Status(405, _)) => {
                    let metadata = self.stat(&cur)?;
                    if !metadata.is_dir {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            format!("WebDAV path exists but is not a directory: {cur}"),
                        ));
                    }
                }
                Err(error) => return Err(request_err(error)),
            }
        }
        Ok(())
    }

    fn parallelism(&self) -> usize {
        2 // HTTP keep-alive; a couple of concurrent requests are fine
    }

    fn rename_overwrites(&self) -> bool {
        // RFC 4918 MOVE Overwrite:T deletes the destination before moving the
        // source; it is not an old-or-new atomic replacement guarantee.
        false
    }

    fn provides_content_hash(&self) -> bool {
        // Nextcloud/ownCloud expose an MD5 via the `oc:checksums` PROPFIND prop
        // (parsed into `content_md5`) — a free content hash, no download. Servers
        // that don't send one leave `content_md5` None, so those files simply
        // fall back to the size+mtime compare (graceful per-file degradation).
        true
    }
}
