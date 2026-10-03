//! WebDAV's optional V1 features: explicit omissions, checked source time
//! and a partial root-ETag poll hint, without an invented server fsync.
use super::core_impl::{request_err, WebdavBackend};
use super::multistatus::parse_multistatus_tolerant;
use super::stream_put::StreamPut;
use crate::connect::{poll_subscription, PollNotice};
use crate::vfs::{Backend, BackendExtensions, ChangeNotice, ChangeSignalMode, ChangeSubscription,
    MtimePrecision, StageFinish, StageFinished, TargetLimits, VfsListing, VfsResult};
use std::io::{self, Write};
use std::sync::atomic::Ordering;
use std::time::Duration;

impl BackendExtensions for WebdavBackend {
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        let listing = parse_multistatus_tolerant(&self.propfind(path, "1")?, path)?;
        if listing.entries.iter().any(|entry| entry.content_md5.is_some()) {
            self.hashes_observed.store(true, Ordering::Relaxed);
        }
        Ok(listing)
    }

    fn target_limits(&self, _root: &str) -> TargetLimits {
        TargetLimits { mtime_precision: MtimePrecision::Seconds, ..TargetLimits::default() }
    }

    fn open_write_copy_stage_timed(&self, path: &str, size: u64, mtime_ms: i64) -> VfsResult<Box<dyn Write + Send>> {
        let agent = if size == 0 { self.mutation_agent.clone() } else { self.write_agent.clone() };
        Ok(Box::new(StreamPut::start_timed(agent, self.url_for(path), self.auth.clone(), size, Some(mtime_ms))?))
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let Some(ms) = finish.mtime_ms else { return Ok(StageFinished::default()) };
        let seconds = ms.div_euclid(1_000);
        let metadata = self.stat(stage)?;
        if metadata.is_dir || metadata.is_symlink || metadata.special {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "WebDAV stage must be a regular file"));
        }
        let mut applied = metadata.mtime_ms.div_euclid(1_000) == seconds && metadata.mtime_ms != 0;
        if !applied && seconds > 86_400 {
            // Nextcloud uses DAV:lastmodified, not the protected
            // getlastmodified. Generic DAV may store a dead property, so
            // even a successful 207 must be followed by effective stat.
            let checksum = metadata.content_md5.as_deref().map(|md5| format!(
                "<oc:checksums><oc:checksum>MD5:{md5}</oc:checksum></oc:checksums>")).unwrap_or_default();
            let body = format!("<d:propertyupdate xmlns:d=\"DAV:\" xmlns:oc=\"http://owncloud.org/ns\"><d:set><d:prop><d:lastmodified>{seconds}</d:lastmodified>{checksum}</d:prop></d:set></d:propertyupdate>");
            let request = self.auth_req(self.write_agent.request("PROPPATCH", &self.url_for(stage))
                .set("Content-Type", "application/xml"));
            match request.send_string(&body) {
                Ok(_) | Err(ureq::Error::Status(403 | 405 | 409 | 501, _)) => {}
                Err(error) => return Err(request_err(error)),
            }
            applied = self.stat(stage)?.mtime_ms.div_euclid(1_000) == seconds;
        }
        if applied { self.remember_stage_time(stage, ms)?; }
        Ok(StageFinished { mtime_applied: applied, durable: false })
    }

    fn change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        Ok(self.root_etag(root)?.map(|_| ChangeSignalMode::Poll))
    }

    fn change_signal(&self, root: &str, interval: Duration,
        tx: crossbeam_channel::Sender<ChangeNotice>) -> VfsResult<Option<ChangeSubscription>> {
        let Some(mut etag) = self.root_etag(root)? else { return Ok(None) };
        let backend = self.clone();
        let root = root.to_string();
        poll_subscription(interval, tx, false, move |_canceled| {
            let current = backend.root_etag(&root)?.ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported,
                "WebDAV root ETag became unavailable"))?;
            if current == etag { return Ok(PollNotice::Quiet) }
            etag = current;
            Ok(PollNotice::Changed)
        }).map(Some)
    }
}

impl WebdavBackend {
    fn root_etag(&self, root: &str) -> VfsResult<Option<String>> {
        let xml = self.propfind(root, "0")?;
        super::metadata::parse(&xml, root).map(|(_, etag)| etag)
    }

    pub(super) fn mkdir_below_root(&self, path: &str) -> VfsResult<()> {
        let root = self.root.trim_end_matches('/');
        let path = path.trim_end_matches('/');
        if path == root {
            if !self.stat(&self.root)?.is_dir {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "WebDAV root is not a collection"));
            }
            return Ok(())
        }
        let relative = path.strip_prefix(root).and_then(|tail| tail.strip_prefix('/'))
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied,
                "WebDAV mkdir lies outside the chosen root"))?;
        let mut current = if root.is_empty() { String::new() } else { root.to_string() };
        for child in relative.split('/').filter(|child| !child.is_empty()) {
            crate::vfs::validate_child_name(child)?;
            current.push('/');
            current.push_str(child);
            self.create_dir(&current)?;
        }
        Ok(())
    }
}
