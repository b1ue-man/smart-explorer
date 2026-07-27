use std::{io, sync::MutexGuard};

use crate::mount::MountEngine;

use super::{handle_access::invalid_handle, handle_state::HandleTable, handle_types::NodeHandle};

/// A reserved-but-unbound handle record. It deliberately does NOT hold the
/// table's namespace-transition lock: a reservation stays open across the
/// engine's whole-file materialization, and holding that lock here would
/// serialize every CreateFile on the drive behind one slow transfer. Delete
/// and rename bookkeeping observe the already-inserted record instead.
pub(super) struct HandleReservation<'a> {
    table: &'a HandleTable,
    key: u64,
    granted_access: u32,
    committed: bool,
}

impl<'a> HandleReservation<'a> {
    pub(super) fn new(table: &'a HandleTable, key: u64, granted_access: u32) -> Self {
        Self {
            table,
            key,
            granted_access,
            committed: false,
        }
    }

    pub(super) fn key(&self) -> u64 {
        self.key
    }

    /// The concrete access rights stored for this handle after
    /// MAXIMUM_ALLOWED resolution.
    pub(super) fn granted_access(&self) -> u32 {
        self.granted_access
    }

    pub(super) fn bind(&self, node: NodeHandle) -> io::Result<()> {
        self.table.bind_reserved(self.key, node)
    }

    pub(super) fn request_delete_and_commit(
        mut self,
        engine: &MountEngine,
        is_directory: bool,
    ) -> io::Result<u64> {
        let path = self.table.reserved_path(self.key)?;
        self.table
            .request_delete(engine, self.key, &path, is_directory)?;
        Ok(self.finish())
    }

    pub(super) fn commit(mut self) -> u64 {
        self.finish()
    }

    fn finish(&mut self) -> u64 {
        self.committed = true;
        self.key
    }
}

impl Drop for HandleReservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.table.abort_reservation(self.key);
        }
    }
}

pub(super) struct RenameReservation<'a> {
    table: &'a HandleTable,
    transition: Option<MutexGuard<'a, ()>>,
    source: String,
    destination: String,
    replace_existing: bool,
    destination_is_open: bool,
}

impl<'a> RenameReservation<'a> {
    pub(super) fn new(
        table: &'a HandleTable,
        transition: MutexGuard<'a, ()>,
        source: String,
        destination: String,
        replace_existing: bool,
        destination_is_open: bool,
    ) -> Self {
        Self {
            table,
            transition: Some(transition),
            source,
            destination,
            replace_existing,
            destination_is_open,
        }
    }

    pub(super) fn destination_is_open(&self) -> bool {
        self.destination_is_open
    }

    pub(super) fn commit(mut self) -> io::Result<()> {
        self.table
            .complete_rename(&self.source, &self.destination, self.replace_existing)?;
        self.transition.take();
        Ok(())
    }
}

pub(super) fn reserved_handle_missing() -> io::Error {
    invalid_handle("reserved file handle disappeared")
}
