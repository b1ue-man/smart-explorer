//! The transfer crate's own test binary links neither `vault`, `daemon` nor
//! `connect` (engine), so it binds stand-ins to their ports (D13): no record
//! is sealed (a vault location is refused, never written in plaintext), no
//! job runs in the worker (the caller runs it, as outside the worker), and
//! the storage stand-ins answer the spool and endpoint ports.
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use zeroize::Zeroizing;

use crate::test_ports::{NoSealedSpools, PlainEndpointRules};
use crate::transfer::{
    JobView, RecordContext, RecordKind, TransferMsg, TransferVault, TransferWorker,
};
use crate::vfs::Backend;

fn no_vault() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "no vault is linked into the transfer layer's tests (D13)",
    )
}

struct NoVault;

impl TransferVault for NoVault {
    fn record_context(&self, locations: &[&str]) -> io::Result<Option<RecordContext>> {
        let vault = locations.iter().any(|location| {
            location
                .trim_start()
                .split_once("://")
                .is_some_and(|(scheme, _)| {
                    scheme.eq_ignore_ascii_case("vault")
                        || scheme.eq_ignore_ascii_case("vault-store")
                })
        });
        if vault {
            Err(no_vault())
        } else {
            Ok(None)
        }
    }
    fn check_live(&self, _context: &RecordContext) -> io::Result<()> {
        Err(no_vault())
    }
    fn seal(
        &self,
        _context: &RecordContext,
        _kind: RecordKind,
        _id: &str,
        _plain: &[u8],
    ) -> io::Result<Vec<u8>> {
        Err(no_vault())
    }
    fn open(
        &self,
        _kind: RecordKind,
        _id: &str,
        _sealed: &[u8],
        _max_plain: usize,
    ) -> io::Result<Zeroizing<Vec<u8>>> {
        Err(no_vault())
    }
    fn site_of(&self, _backend: &dyn Backend) -> Option<Option<String>> {
        None
    }
    fn mount_of(&self, _path: &Path) -> Option<Option<String>> {
        None
    }
    fn seal_log_fields(&self, _site: &str, _items: &[&str]) -> Result<Vec<String>, String> {
        Err(no_vault().to_string())
    }
}

struct NoWorker;

impl TransferWorker for NoWorker {
    fn run_in_worker(
        &self,
        _view: &JobView<'_>,
        _report: &(dyn Fn(TransferMsg) + Sync),
        _cancel: &AtomicBool,
    ) -> bool {
        false
    }
}

crate::provide_port!(smart_explorer_port_transfer_vault: dyn TransferVault = NoVault);
crate::provide_port!(smart_explorer_port_transfer_worker: dyn TransferWorker = NoWorker);
crate::provide_port!(smart_explorer_port_vfs_sealed_spools: dyn crate::vfs::SealedSpools = NoSealedSpools);
crate::provide_port!(
    smart_explorer_port_vfs_sync_endpoints: dyn crate::vfs::SyncEndpointRules = PlainEndpointRules
);
