//! WebDAV requests for the transfer engine (plan W1): one MKCOL per folder,
//! a server-side COPY into a private stage, a ranged GET to resume, the
//! streaming stage PUT and removal of an unpublished stage.
use super::super::status::range_start;
use super::super::stream_put::StreamPut;
use super::{request_err, WebdavBackend};
use crate::vfs::{Backend, VfsResult};
use std::io::{self, Read, Write};

impl WebdavBackend {
    /// One MKCOL; `Ok(false)` when the name is taken (405, RFC 4918 §9.3.1).
    fn make_collection(&self, path: &str) -> io::Result<bool> {
        // Send the collection URL initially; never follow a mutation redirect.
        let mut url = self.url_for(path);
        if !url.ends_with('/') {
            url.push('/');
        }
        match self
            .auth_req(self.write_agent.request("MKCOL", &url))
            .call()
        {
            Ok(response) if (200..300).contains(&response.status()) && response.status() != 207 => {
                Ok(true)
            }
            Ok(response) => Err(io::Error::other(format!(
                "WebDAV MKCOL returned unexpected HTTP status {}",
                response.status()
            ))),
            Err(ureq::Error::Status(405, _)) => Ok(false),
            Err(error @ ureq::Error::Status(404 | 409, _)) => {
                // A slash-addressed regular file can look missing or conflict.
                // Only a fresh proof of the original file establishes collision;
                // absent parents, denied probes and transport errors stay errors.
                match self.stat(path) {
                    Ok(meta) if !meta.is_dir => Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{path} existiert bereits"),
                    )),
                    Err(probe) if probe.kind() != io::ErrorKind::NotFound => Err(probe),
                    _ => Err(request_err(error)),
                }
            }
            Err(error) => Err(request_err(error)),
        }
    }

    /// `create_dir` (an existing collection is fine) and `create_dir_new`
    /// (any taken name is `AlreadyExists`).
    pub(super) fn create_collection(&self, path: &str, exclusive: bool) -> VfsResult<()> {
        if self.make_collection(path)? {
            return Ok(());
        }
        if !exclusive && self.stat(path)?.is_dir {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{path} existiert bereits"),
        ))
    }

    /// COPY `src` to the new `stage` without replacing anything; the size of
    /// the copy, or `None` when the server copies nothing itself (405/501) or
    /// not to there (502, RFC 4918 §9.8.5), so the engine streams instead.
    pub(super) fn copy_to_stage(&self, src: &str, stage: &str) -> VfsResult<Option<u64>> {
        let request = self
            .write_agent
            .request("COPY", &self.url_for(src))
            .set("Destination", &self.url_for(stage))
            .set("Overwrite", "F");
        match self.auth_req(request).call() {
            Ok(response) if (200..300).contains(&response.status()) && response.status() != 207 => {
            }
            Ok(response) => {
                return Err(io::Error::other(format!(
                    "WebDAV COPY returned unexpected HTTP status {}",
                    response.status()
                )))
            }
            Err(ureq::Error::Status(405 | 501 | 502, _)) => return Ok(None),
            Err(error) => return Err(request_err(error)),
        }
        Ok(Some(self.stat(stage)?.size))
    }

    /// GET from `offset`; `None` when the server ignores the range (it sends
    /// the whole file) or answers another one: the download starts over.
    pub(super) fn read_from(
        &self,
        path: &str,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        if offset == 0 {
            return self.open_read(path).map(Some);
        }
        let response = self.get_from(path, offset)?;
        let starts_at = if response.status() == 206 {
            response.header("Content-Range").and_then(range_start)
        } else {
            None
        };
        if starts_at != Some(offset) {
            return Ok(None);
        }
        Ok(Some(Box::new(response.into_reader())))
    }

    /// The streaming PUT of a sized stage; an empty one goes over the
    /// unpooled agent (ureq would replay an empty body).
    pub(super) fn stage_writer(&self, path: &str, size: u64) -> VfsResult<Box<dyn Write + Send>> {
        let agent = if size == 0 {
            self.mutation_agent.clone()
        } else {
            self.write_agent.clone()
        };
        let writer = StreamPut::start(agent, self.url_for(path), self.auth.clone(), size)?;
        Ok(Box::new(writer))
    }

    /// Removes an unpublished stage; one that is gone leaves nothing to do.
    pub(super) fn discard_stage(&self, stage: &str) -> VfsResult<()> {
        match self.stat(stage) {
            Ok(meta) if !meta.is_dir => self.remove_file(stage),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Übertragungsstufe {stage} ist keine Datei und bleibt stehen"),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}
