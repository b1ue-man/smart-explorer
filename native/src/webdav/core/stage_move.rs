//! One MOVE request carries the verified stage time and, when available,
//! its strong ETag. Readback never turns MOVE into an atomic replacement.
use super::core_impl::WebdavBackend;
use crate::vfs::{Backend, VfsResult};
use std::io;

impl WebdavBackend {
    pub(super) fn remember_stage_time(&self, path: &str, ms: i64) -> VfsResult<()> {
        let (actual, etag) = super::metadata::parse(&self.propfind(path, "0")?, path)?;
        if actual.mtime_ms.div_euclid(1_000) != ms.div_euclid(1_000) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "WebDAV stage time changed before confirmation"));
        }
        let etag = etag.filter(|tag| !tag.starts_with("W/"));
        let rounded = ms.div_euclid(1_000).checked_mul(1_000).ok_or_else(||
            io::Error::new(io::ErrorKind::InvalidInput, "WebDAV source time is out of range"))?;
        self.stage_times.lock().map_err(|_| io::Error::other("WebDAV stage-time lock poisoned"))?
            .insert(path.to_string(), (rounded, etag));
        Ok(())
    }

    pub(super) fn move_stage_time(&self, src: &str, dst: &str, replace: bool) -> VfsResult<()> {
        let expected = self.stage_times.lock().map_err(|_| io::Error::other("WebDAV stage-time lock poisoned"))?
            .get(src).cloned();
        let mut request = self.write_agent.request("MOVE", &self.url_for(src))
            .set("Destination", &self.url_for(dst)).set("Overwrite", if replace { "T" } else { "F" });
        if let Some((ms, etag)) = &expected {
            request = request.set("X-OC-Mtime", &ms.div_euclid(1_000).to_string());
            if let Some(etag) = etag { request = request.set("If-Match", etag); }
        }
        self.mutation(request, "MOVE")?;
        if let Some((ms, _)) = expected {
            let actual = self.stat(dst)?;
            if actual.mtime_ms.div_euclid(1_000) != ms.div_euclid(1_000) {
                return Err(io::Error::new(io::ErrorKind::Unsupported, format!(
                    "WebDAV MOVE published {dst} but did not retain the confirmed stage time; source was {src}")));
            }
            let mut times = self.stage_times.lock().map_err(|_| io::Error::other("WebDAV stage-time lock poisoned"))?;
            times.remove(src);
        }
        Ok(())
    }
}
