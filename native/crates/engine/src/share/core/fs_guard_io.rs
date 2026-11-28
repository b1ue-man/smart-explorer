//! Readers and writers of a guarded backend check the live session around
//! every block, so a revoked right ends a running transfer.
use super::super::fs_access::AccessAuthority;
use std::{
    io::{self, Read, Write},
    sync::Arc,
};

pub(super) struct GuardedReader {
    pub(super) inner: Box<dyn Read + Send>,
    pub(super) authority: Option<Arc<AccessAuthority>>,
}
impl Read for GuardedReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if let Some(authority) = &self.authority {
            authority.check()?;
        }
        let read = self.inner.read(bytes)?;
        if let Some(authority) = &self.authority {
            authority.check()?;
        }
        Ok(read)
    }
}
pub(super) struct GuardedWriter {
    pub(super) inner: Box<dyn Write + Send>,
    pub(super) authority: Option<Arc<AccessAuthority>>,
}
impl Write for GuardedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(authority) = &self.authority {
            authority.check_write()?;
        }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(authority) = &self.authority {
            authority.check_write()?;
        }
        self.inner.flush()
    }
}
