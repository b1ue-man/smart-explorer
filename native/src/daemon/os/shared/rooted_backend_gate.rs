use std::io;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub(super) struct OperationGate(RwLock<()>);

impl OperationGate {
    pub(super) fn new() -> Self {
        Self(RwLock::new(()))
    }

    pub(super) fn read(&self) -> io::Result<RwLockReadGuard<'_, ()>> {
        self.0
            .read()
            .map_err(|_| super::mount_error::encoded(io::ErrorKind::Other))
    }

    pub(super) fn write(&self) -> io::Result<RwLockWriteGuard<'_, ()>> {
        self.0
            .write()
            .map_err(|_| super::mount_error::encoded(io::ErrorKind::Other))
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
