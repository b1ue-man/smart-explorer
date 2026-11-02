//! The vault side of the transfer layer's vault port (D13): sealing of
//! persisted transfer records and transfer logs, the vault site of a backend
//! and the vault drive of a local path. Every call is the vault function of
//! the same name; nothing here weakens or bypasses it.
use std::io;
use std::path::Path;

use zeroize::Zeroizing;

use super::{
    check_transfer_blob_context_live, open_transfer_blob, seal_log_fields, seal_transfer_blob,
    transfer_blob_context, vault_mount_of, vault_site_of, TransferBlobPurpose,
};
use crate::transfer::ports::{RecordContext, TransferVault};
use crate::transfer::RecordKind;
use crate::vfs::Backend;

struct VaultTransfers;

fn purpose(kind: RecordKind) -> TransferBlobPurpose {
    match kind {
        RecordKind::Job => TransferBlobPurpose::Job,
        RecordKind::Alias => TransferBlobPurpose::Alias,
    }
}

impl TransferVault for VaultTransfers {
    fn record_context(&self, locations: &[&str]) -> io::Result<Option<RecordContext>> {
        transfer_blob_context(locations)
    }

    fn check_live(&self, context: &RecordContext) -> io::Result<()> {
        check_transfer_blob_context_live(context)
    }

    fn seal(
        &self,
        context: &RecordContext,
        kind: RecordKind,
        id: &str,
        plain: &[u8],
    ) -> io::Result<Vec<u8>> {
        seal_transfer_blob(context, purpose(kind), id, plain)
    }

    fn open(
        &self,
        kind: RecordKind,
        id: &str,
        sealed: &[u8],
        max_plain: usize,
    ) -> io::Result<Zeroizing<Vec<u8>>> {
        open_transfer_blob(purpose(kind), id, sealed, max_plain)
    }

    fn site_of(&self, backend: &dyn Backend) -> Option<Option<String>> {
        vault_site_of(backend)
    }

    fn mount_of(&self, path: &Path) -> Option<Option<String>> {
        vault_mount_of(path)
    }

    fn seal_log_fields(&self, site: &str, items: &[&str]) -> Result<Vec<String>, String> {
        seal_log_fields(site, items)
    }
}

crate::provide_port!(smart_explorer_port_transfer_vault: dyn TransferVault = VaultTransfers);
