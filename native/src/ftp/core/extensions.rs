use super::core_impl::FtpBackend;
use super::errors::{command_path, map, reply_code};
use super::metadata::{list, read_time};
use crate::vfs::{
    Backend, BackendExtensions, StageFinish, StageFinished, TargetLimits, VfsListing, VfsResult,
};
use suppaftp::Status;

impl BackendExtensions for FtpBackend {
    fn replace_staged_reversible(
        &self,
        _staged: &str,
        _destination: &str,
        _retained: &str,
    ) -> VfsResult<bool> {
        // RFC 959 supplies no NoReplace RNTO or conditional restore. A
        // third party could fill either slot after an absence probe; no
        // portable reversible replacement is available before mutation.
        Ok(false)
    }
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        self.pool.retire_idle();
        list(self.pool.primary(), path)
    }

    fn target_limits(&self, _root: &str) -> TargetLimits {
        TargetLimits {
            mtime_precision: self.pool.primary().observed_precision(),
            ..TargetLimits::default()
        }
    }

    fn finish_stage(&self, path: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        command_path(path)?;
        let metadata = self.stat(path)?;
        if metadata.is_dir || metadata.is_symlink || metadata.special {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "FTP stage must be a regular file",
            ));
        }
        let Some(ms) = finish.mtime_ms else {
            return Ok(StageFinished::default());
        };
        let Some(date) = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms) else {
            return Ok(StageFinished::default());
        };
        if !self.pool.primary().features()?.mfmt {
            return Ok(StageFinished::default());
        }
        let stamp = date.format("%Y%m%d%H%M%S").to_string();
        let applied = self.pool.primary().with_stream_mutation(|stream| {
            match stream.custom_command(format!("MFMT {stamp} {path}"), &[Status::File]) {
                Ok(_) => {}
                Err(error) if matches!(reply_code(&error), Some(500 | 501 | 502 | 504 | 550)) => {
                    return Ok(false)
                }
                Err(error) => return Err(map(error)),
            }
            Ok(read_time(stream, path)?.is_some_and(|actual| {
                actual == ms
                    || (self.pool.primary().observed_precision()
                        != crate::vfs::MtimePrecision::Unknown
                        && actual.div_euclid(1_000) == ms.div_euclid(1_000))
            }))
        })?;
        // FTP offers no filesystem flush primitive. A confirmed 226/250 is
        // never presented as fsync or durable namespace publication.
        Ok(StageFinished {
            mtime_applied: applied,
            durable: false,
        })
    }
}
