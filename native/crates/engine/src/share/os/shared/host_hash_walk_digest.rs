//! The requested digest of one regular file of a host hash walk, read from
//! a reader the walk opened; without an algorithm nothing is read.
use std::io::{self, Read};
use std::sync::atomic::Ordering;

use sha2::{Digest, Sha256};

use super::Shared;
use crate::share::wire::FsHashAlgo;

impl Shared<'_> {
    pub(super) fn digest(&self, mut read: impl Read, expected: u64) -> io::Result<Option<String>> {
        if self.request.algo.is_none() {
            return Ok(None);
        }
        let mut sha = Sha256::new();
        let mut md5 = md5::Context::new();
        let mut buffer = vec![0; 1024 * 1024];
        let mut size = 0u64;
        loop {
            self.check()?;
            let n = match read.read(&mut buffer) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if n == 0 {
                break;
            }
            size = size.saturating_add(n as u64);
            if size > expected {
                break;
            }
            match self.request.algo {
                Some(FsHashAlgo::Sha256) => sha.update(&buffer[..n]),
                Some(FsHashAlgo::Md5) => md5.consume(&buffer[..n]),
                _ => {}
            }
            self.progress.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
        if size != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Dateigröße hat sich beim Lesen geändert",
            ));
        }
        Ok(Some(match self.request.algo {
            Some(FsHashAlgo::Sha256) => format!("{:x}", sha.finalize()),
            Some(FsHashAlgo::Md5) => format!("{:x}", md5.compute()),
            _ => return Ok(None),
        }))
    }
}

/// The app's digest cache folder (none in tests).
pub(super) fn cache_folder() -> Option<std::path::PathBuf> {
    (!cfg!(any(test, feature = "test-support"))).then(crate::support_dirs::app_data_dir)
}
