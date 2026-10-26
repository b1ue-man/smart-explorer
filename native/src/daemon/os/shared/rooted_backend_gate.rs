use std::io::{self, Read, Write};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub(super) struct OperationGate(RwLock<()>, Option<crate::vault::MountAccess>);

impl OperationGate {
    pub(super) fn new() -> Self {
        Self(RwLock::new(()), None)
    }

    pub(super) fn authorized(access: Option<crate::vault::MountAccess>) -> Self {
        Self(RwLock::new(()), access)
    }

    fn check(&self) -> io::Result<()> {
        self.1.as_ref().map_or(Ok(()), crate::vault::MountAccess::check)
    }

    pub(super) fn reader(&self, inner: Box<dyn Read + Send>) -> Box<dyn Read + Send> {
        Box::new(Reader { inner, access: self.1.clone() })
    }

    pub(super) fn writer(&self, inner: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
        Box::new(Writer { inner, access: self.1.clone() })
    }

    pub(super) fn read(&self) -> io::Result<RwLockReadGuard<'_, ()>> {
        let held = self.0.read()
            .map_err(|_| super::mount_error::encoded(io::ErrorKind::Other))?;
        self.check()?;
        Ok(held)
    }

    pub(super) fn write(&self) -> io::Result<RwLockWriteGuard<'_, ()>> {
        let held = self.0.write()
            .map_err(|_| super::mount_error::encoded(io::ErrorKind::Other))?;
        self.check()?;
        Ok(held)
    }
}

struct Reader {
    inner: Box<dyn Read + Send>,
    access: Option<crate::vault::MountAccess>,
}

impl Read for Reader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if let Some(access) = &self.access { access.check()?; }
        self.inner.read(bytes)
    }
}

struct Writer {
    inner: Box<dyn Write + Send>,
    access: Option<crate::vault::MountAccess>,
}

impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(access) = &self.access { access.check()?; }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(access) = &self.access { access.check()?; }
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_rooted_reads_share_gate_but_mutation_waits() {
        let gate = OperationGate::new();
        let first = gate.read().unwrap();
        let second = gate.0.try_read().expect("parallel read must enter");
        assert!(gate.0.try_write().is_err());
        drop(second);
        drop(first);
        assert!(gate.0.try_write().is_ok());
    }
}
