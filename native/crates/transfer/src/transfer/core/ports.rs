//! The ports of the transfer layer (plan D13): what the transfer engine needs
//! from the vaults (`vault`) and from the background worker (`daemon`), both
//! in higher layers. Their implementations are bound through `crate::ports`
//! by the linker, so no program and no test runs a transfer without them.
use std::any::Any;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use zeroize::Zeroizing;

use super::types::TransferMsg;
use super::{JobView, RecordKind};
use crate::vfs::Backend;

/// K20: what an issue line of a vault says when its fields cannot be sealed
/// (`vault::FIELDS_WITHHELD` re-exports it).
pub const FIELDS_WITHHELD: &str =
    "Eintrag eines Tresors – Pfad und Meldung nur bei entsperrtem Tresor in der Übertragungsliste";

/// The vault scope a persisted transfer record is sealed for. Only the vault
/// port creates and reads its contents (`vault::TransferBlobContext` is this
/// type); the transfer layer only keeps and passes it. A context the port
/// did not create is refused by every port call, never treated as "no
/// vault". Never serialized; Debug shows no contents.
#[derive(Clone)]
pub struct RecordContext(Arc<dyn Any + Send + Sync>);

impl RecordContext {
    /// Wraps the scope the vault port captured.
    pub fn new<T: Any + Send + Sync>(scope: T) -> Self {
        Self(Arc::new(scope))
    }

    /// The captured scope; `None` when it is not of type `T`.
    pub fn scope<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl std::fmt::Debug for RecordContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TransferBlobContext(..)")
    }
}

/// Sealing of persisted transfer records and logs with the vault keys
/// (K20, plan 2026-10-17). No call falls back to plaintext: every failure
/// is an error the caller reports or turns into a withheld field.
pub trait TransferVault: Sync {
    /// The vault scope of `locations` (`vault::transfer_blob_context`):
    /// `None` when no location lies in a vault, an error when a vault
    /// binding cannot be confirmed.
    fn record_context(&self, locations: &[&str]) -> io::Result<Option<RecordContext>>;
    /// Whether the session epochs and bindings of `context` still hold
    /// (`vault::check_transfer_blob_context_live`).
    fn check_live(&self, context: &RecordContext) -> io::Result<()>;
    /// Seals one record (`vault::seal_transfer_blob`).
    fn seal(
        &self,
        context: &RecordContext,
        kind: RecordKind,
        id: &str,
        plain: &[u8],
    ) -> io::Result<Vec<u8>>;
    /// Opens a sealed record (`vault::open_transfer_blob`); no bytes on any
    /// changed purpose, record, binding or unlock state.
    fn open(
        &self,
        kind: RecordKind,
        id: &str,
        sealed: &[u8],
        max_plain: usize,
    ) -> io::Result<Zeroizing<Vec<u8>>>;
    /// `Some(site)` for a vault backend opened here, `Some(None)` for one of
    /// unknown site, `None` for every other backend (`vault::vault_site_of`).
    fn site_of(&self, backend: &dyn Backend) -> Option<Option<String>>;
    /// `Some(site)` when `path` lies in a vault drive, `Some(None)` for a
    /// vault drive of unknown site (`vault::vault_mount_of`).
    fn mount_of(&self, path: &Path) -> Option<Option<String>>;
    /// Seals log fields of `site` (`vault::seal_log_fields`).
    fn seal_log_fields(&self, site: &str, items: &[&str]) -> Result<Vec<String>, String>;
}

crate::declare_port! {
    /// The vault implementation (provided by `vault`, os/shared/transfer_port.rs).
    pub fn vault() -> dyn TransferVault = smart_explorer_port_transfer_vault;
}

/// Hands a job to the background worker (Z-P2).
pub trait TransferWorker: Sync {
    /// Runs `view` in the worker when it qualifies; `false` when nothing
    /// was started there and the caller runs the job itself. `true` means
    /// `report` received exactly one `Done`
    /// (`daemon::run_transfer_in_worker`).
    fn run_in_worker(
        &self,
        view: &JobView<'_>,
        report: &(dyn Fn(TransferMsg) + Sync),
        cancel: &AtomicBool,
    ) -> bool;
}

crate::declare_port! {
    /// The worker implementation (provided by `daemon`,
    /// os/shared/transfer_job_client.rs).
    pub fn worker() -> dyn TransferWorker = smart_explorer_port_transfer_worker;
}
