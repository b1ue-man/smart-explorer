//! The ports of the storage layer (plan D13): what `vfs` needs from modules
//! in higher layers, bound through `crate::ports` by the linker, so no
//! program and no test runs `vfs` without them.
//!
//! - `UrlBackends`: an `sftp://` or `smb://` root opens through the SFTP and
//!   SMB backends (`sftp`, `smb`; transfer layer).
//! - `SealedSpools`: the spool of a copy with a vault side holds only
//!   ciphertext under a key used once (K08; `vault`).
//! - `SyncEndpointRules`: the endpoint rules of every general sync (no vault
//!   side, distinct and non-nested endpoints; `connect`).
//! - `LocalServerCopy`: the kernel copy of one local file into a new stage
//!   (`LocalBackend::server_copy_to_stage`; `copy`).
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::BackendHandle;

/// Opens the backend of one URL scheme from a root URL with its
/// credentials (`sftp::backend_from_url`, `smb::backend_from_url`).
pub trait UrlBackends: Sync {
    fn open(&self, url: &str) -> io::Result<BackendHandle>;
}

crate::declare_port! {
    /// `sftp://` roots (provided by `sftp`, core/url.rs).
    pub fn sftp_backends() -> dyn UrlBackends = smart_explorer_port_vfs_sftp;
}

crate::declare_port! {
    /// `smb://` roots (provided by `smb`, core/url.rs).
    pub fn smb_backends() -> dyn UrlBackends = smart_explorer_port_vfs_smb;
}

/// Creates the encrypted spool of one copy (`vault::EphemeralSpool`).
pub trait SealedSpools: Sync {
    fn create(&self) -> io::Result<Box<dyn SealedSpool>>;
}

/// An anonymous spool file that holds the copied bytes only encrypted under
/// a key that exists in memory for this one copy.
pub trait SealedSpool: Send {
    /// Fills the spool from `reader` (from its start); the cleartext bytes
    /// taken.
    fn fill(&mut self, reader: &mut dyn Read) -> io::Result<u64>;
    /// The cleartext from the start; every chunk is authenticated.
    fn reader(&mut self) -> io::Result<Box<dyn Read + '_>>;
}

crate::declare_port! {
    /// The vault spool (provided by `vault`, os/shared/ephemeral_spool.rs).
    pub fn sealed_spools() -> dyn SealedSpools = smart_explorer_port_vfs_sealed_spools;
}

/// Why a general sync refuses a vault locator as one of its sides: the one
/// text of every implementation of [`SyncEndpointRules`] (`connect`
/// re-exports it as `VAULT_SIDE_REFUSED`).
pub const VAULT_SIDE_REFUSED: &str = "Tresororte (vault://, vault-store://) sind keine Seite allgemeiner Sync-Jobs: Tresore gleicht der Tresor-Sync ab";

/// The endpoint rules of a general sync (`connect::validate_sync_endpoints`).
pub trait SyncEndpointRules: Sync {
    /// Vault locators are no side of a general sync; source and target must
    /// be distinct and not nested. The error is the text for the user.
    fn validate(&self, source: &str, target: &str) -> Result<(), String>;
}

crate::declare_port! {
    /// The rules of `connect` (provided there, core/location.rs).
    pub fn sync_endpoint_rules() -> dyn SyncEndpointRules = smart_explorer_port_vfs_sync_endpoints;
}

/// `connect::validate_sync_endpoints` for the storage and transfer layers:
/// pure endpoint validation shared by the editor, persistence and daemon.
pub fn validate_sync_endpoints(source: &str, target: &str) -> Result<(), String> {
    sync_endpoint_rules().validate(source, target)
}

/// The kernel copy of one regular local file into a new file
/// (`copy::copy_to_new_file`).
pub trait LocalServerCopy: Sync {
    /// Copies `source` into the new file `stage` and returns its length.
    /// `stage` must not exist; a link, junction or special file as source is
    /// refused. The result is a plain regular file of exactly `expected`
    /// bytes from a source that did not change meanwhile, otherwise it is
    /// removed again and the copy fails.
    fn copy_to_new_file(
        &self,
        source: &Path,
        stage: &Path,
        expected: u64,
        cancel: &AtomicBool,
    ) -> io::Result<u64>;
}

crate::declare_port! {
    /// The kernel copy of `copy` (provided there, os/shared/server_copy.rs).
    pub fn local_server_copy() -> dyn LocalServerCopy = smart_explorer_port_vfs_local_server_copy;
}
