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

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
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

/// Percent-encode a path, preserving `/`. Unreserved chars pass through.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Percent-decode a path component.
fn decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The path portion of an href that may be an absolute URL or absolute path.
fn href_path(href: &str) -> String {
    let p = if let Some(rest) = href.split_once("://") {
        // strip scheme://authority
        match rest.1.find('/') {
            Some(i) => rest.1[i..].to_string(),
            None => "/".to_string(),
        }
    } else {
        href.to_string()
    };
    decode_path(&p)
}

fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

/// Parse a WebDAV `multistatus` body into entries, dropping the `self` entry
/// (the listed directory itself, whose path equals `request_path`).
fn parse_multistatus(xml: &str, request_path: &str) -> Vec<VfsMeta> {
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let want = request_path.trim_end_matches('/');
    let mut out = Vec::new();
    for resp in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "response")
    {
        let href = resp
            .descendants()
            .find(|n| n.tag_name().name() == "href")
            .and_then(|n| n.text())
            .unwrap_or("");
        if href.is_empty() {
            continue;
        }
        let path = href_path(href);
        if path.trim_end_matches('/') == want {
            continue; // the directory itself
        }
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
        // ownCloud/Nextcloud checksums: "<oc:checksums><oc:checksum>SHA1:… MD5:… ADLER32:…</oc:checksum></oc:checksums>".
        let content_md5 = resp
            .descendants()
            .find(|n| n.tag_name().name() == "checksums")
            .and_then(|cs| {
                cs.descendants()
                    .find_map(|d| d.text().and_then(extract_md5))
            });
        let name = basename(&path);
        if name.is_empty() {
            continue;
        }
        out.push(VfsMeta {
            is_dir,
            is_symlink: false,
            size: if is_dir { 0 } else { size },
            mtime_ms,
            btime_ms: 0,
            hidden: name.starts_with('.'),
            system: false,
            name,
            id: None,
            content_md5,
        });
    }
    out
}

/// Extract the MD5 hex from an ownCloud/Nextcloud checksum string like
/// "SHA1:… MD5:<hex> ADLER32:…".
fn extract_md5(s: &str) -> Option<String> {
    s.split_whitespace().find_map(|tok| {
        let (k, v) = tok.split_once(':')?;

        if k.eq_ignore_ascii_case("MD5")
            && v.len() == 32
            && v.bytes().all(|b| b.is_ascii_hexdigit())
        {
            Some(v.to_string())
        } else {
            None
        }
    })
}

/// RFC 1123 date ("Mon, 01 Jan 2024 12:00:00 GMT") → unix ms.
fn parse_http_date_ms(s: &str) -> Option<i64> {
    let dt = chrono::NaiveDateTime::parse_from_str(s.trim(), "%a, %d %b %Y %H:%M:%S GMT").ok()?;
    Some(dt.and_utc().timestamp_millis())
}

pub struct WebdavBackend {
    base: String, // scheme://host:port
    root: String, // forward-slash path
    auth: String, // "Basic ..." (empty = none)
    agent: ureq::Agent,
    /// Display label, consumed by the connect-UI step.
    #[allow(dead_code)]
    url: String,
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
        let be = WebdavBackend {
            url: format!("webdav {}{}", base, root),
            base,
            root: root.clone(),
            auth,
            agent,
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
            .map_err(io_err)?
            .into_string()
            .map_err(io_err)
    }
}

impl Backend for WebdavBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Webdav
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let xml = self.propfind(path, "1")?;
        Ok(parse_multistatus(&xml, path))
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let xml = self.propfind(path, "0")?;
        // Depth 0 returns the resource itself; parse without dropping self.
        let doc = roxmltree::Document::parse(&xml).map_err(io_err)?;
        let resp = doc
            .descendants()
            .find(|n| n.tag_name().name() == "response")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "kein PROPFIND-Response"))?;
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
            .map_err(io_err)?;
        Ok(Box::new(resp.into_reader()))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(WebdavWriter {
            agent: self.agent.clone(),
            url: self.url_for(path),
            auth: self.auth.clone(),
            buf: Vec::new(),
            committed: false,
        }))
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        // Server-side COPY.
        self.auth_req(
            self.agent
                .request("COPY", &self.url_for(src))
                .set("Destination", &self.url_for(dst))
                .set("Overwrite", "T"),
        )
        .call()
        .map_err(io_err)?;
        Ok(self.stat(dst).map(|m| m.size).unwrap_or(0))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.auth_req(
            self.agent
                .request("MOVE", &self.url_for(src))
                .set("Destination", &self.url_for(dst))
                .set("Overwrite", "T"),
        )
        .call()
        .map_err(io_err)?;
        Ok(())
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.auth_req(self.agent.request("DELETE", &self.url_for(path)))
            .call()
            .map_err(io_err)?;
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
            // MKCOL; ignore "already exists" (405/409 surface as Err -> swallow).
            let _ = self
                .auth_req(self.agent.request("MKCOL", &self.url_for(&cur)))
                .call();
        }
        Ok(())
    }

    fn parallelism(&self) -> usize {
        2 // HTTP keep-alive; a couple of concurrent requests are fine
    }

    fn rename_overwrites(&self) -> bool {
        true
    }

    fn provides_content_hash(&self) -> bool {
        // Nextcloud/ownCloud expose an MD5 via the `oc:checksums` PROPFIND prop
        // (parsed into `content_md5`) — a free content hash, no download. Servers
        // that don't send one leave `content_md5` None, so those files simply
        // fall back to the size+mtime compare (graceful per-file degradation).
        true
    }
}

struct WebdavWriter {
    agent: ureq::Agent,
    url: String,
    auth: String,
    buf: Vec<u8>,
    committed: bool,
}

impl WebdavWriter {
    fn commit(&mut self) -> io::Result<()> {
        if self.committed {
            return Ok(());
        }
        self.committed = true;
        let data = std::mem::take(&mut self.buf);
        let req = self.agent.put(&self.url);
        let req = if self.auth.is_empty() {
            req
        } else {
            req.set("Authorization", &self.auth)
        };
        req.send_bytes(&data).map_err(io_err)?;
        Ok(())
    }
}

impl Write for WebdavWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.committed {
            return Err(io_err("Upload bereits abgeschlossen"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.commit()
    }
}

impl Drop for WebdavWriter {
    fn drop(&mut self) {
        let _ = self.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/dav/files/me/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/files/me/notes.txt</d:href>
    <d:propstat><d:prop>
      <d:resourcetype/>
      <d:getcontentlength>1234</d:getcontentlength>
      <d:getlastmodified>Mon, 01 Jan 2024 12:00:00 GMT</d:getlastmodified>
    </d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/files/me/sub%20dir/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn parse_skips_self_and_reads_props() {
        let entries = parse_multistatus(SAMPLE, "/dav/files/me");
        assert_eq!(entries.len(), 2, "self entry must be dropped");
        let f = entries.iter().find(|e| e.name == "notes.txt").unwrap();
        assert!(!f.is_dir);
        assert_eq!(f.size, 1234);
        assert!(f.mtime_ms > 0);
        let d = entries.iter().find(|e| e.name == "sub dir").unwrap();
        assert!(d.is_dir);
        assert_eq!(d.size, 0);
    }

    #[test]
    fn path_encode_decode() {
        assert_eq!(encode_path("/a b/c.txt"), "/a%20b/c.txt");
        assert_eq!(decode_path("/a%20b/c.txt"), "/a b/c.txt");
        assert_eq!(href_path("https://host:8443/dav/x%20y"), "/dav/x y");
        assert_eq!(href_path("/dav/z"), "/dav/z");
    }

    #[test]
    fn http_date_parses() {
        assert!(parse_http_date_ms("Mon, 01 Jan 2024 12:00:00 GMT").unwrap() > 0);
        assert!(parse_http_date_ms("garbage").is_none());
    }
}
