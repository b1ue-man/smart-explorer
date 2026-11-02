use std::io::{self, Read, Write};

use crate::vfs::VfsMeta;

use super::mount_error::encode as sanitize_error;

pub(super) fn sanitize_metadata(mut metadata: VfsMeta) -> VfsMeta {
    metadata.id = None;
    metadata.content_md5 = metadata.content_md5.and_then(|hash| {
        (hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    });
    metadata
}

pub(super) struct SanitizedReader {
    pub(super) inner: Box<dyn Read + Send>,
}

impl Read for SanitizedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer).map_err(sanitize_error)
    }
}

pub(super) struct SanitizedWriter {
    pub(super) inner: Box<dyn Write + Send>,
}

impl Write for SanitizedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer).map_err(sanitize_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().map_err(sanitize_error)
    }
}
