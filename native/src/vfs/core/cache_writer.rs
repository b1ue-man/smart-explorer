use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use super::cache_support::invalidate_shared;
use super::CacheState;

pub(super) struct InvalidatingWriter {
    inner: Option<Box<dyn Write + Send>>,
    cache: Arc<Mutex<CacheState>>,
    path: String,
}

impl InvalidatingWriter {
    pub(super) fn new(
        inner: Box<dyn Write + Send>,
        cache: Arc<Mutex<CacheState>>,
        path: &str,
    ) -> Self {
        Self {
            inner: Some(inner),
            cache,
            path: path.to_string(),
        }
    }

    fn inner(&mut self) -> io::Result<&mut Box<dyn Write + Send>> {
        self.inner
            .as_mut()
            .ok_or_else(|| io::Error::other("cached backend writer is already closed"))
    }

    fn invalidate(&self) {
        invalidate_shared(&self.cache, &self.path);
    }
}

impl Write for InvalidatingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner()?.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.inner()?.flush();
        self.invalidate();
        result
    }
}

impl Drop for InvalidatingWriter {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            drop(inner);
            self.invalidate();
        }
    }
}
