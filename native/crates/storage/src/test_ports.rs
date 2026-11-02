//! Stand-ins for the ports of the storage layer in the tests of the storage
//! and transfer crates (plan docs/plaene/2026-11-01-idealzustand, D13).
//! Their test binaries do not link the higher modules that provide the real
//! implementations (`crate::ports`), so these stand-ins refuse what needs
//! them: no SFTP/SMB root opens, no vault spool and no kernel copy exist
//! there. The endpoint rules keep their observable checks without `connect`:
//! a vault locator is no side of a general sync, and the sides must be
//! distinct and not nested. Tests of the real behaviour live in the crates
//! that link the providers (engine and above).
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::vfs::{
    location_paths, BackendHandle, LocalServerCopy, SealedSpool, SealedSpools, SyncEndpointRules,
    UrlBackends,
};

fn unavailable(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} is not linked into the tests of this layer (D13)"),
    )
}

/// `sftp://` and `smb://` roots.
pub struct NoUrlBackends;

impl UrlBackends for NoUrlBackends {
    fn open(&self, url: &str) -> io::Result<BackendHandle> {
        Err(unavailable(url.split("://").next().unwrap_or(url)))
    }
}

/// The vault spool.
pub struct NoSealedSpools;

impl SealedSpools for NoSealedSpools {
    fn create(&self) -> io::Result<Box<dyn SealedSpool>> {
        Err(unavailable("the vault spool"))
    }
}

/// The kernel copy of `copy`.
pub struct NoLocalServerCopy;

impl LocalServerCopy for NoLocalServerCopy {
    fn copy_to_new_file(
        &self,
        _source: &Path,
        _stage: &Path,
        _expected: u64,
        _cancel: &AtomicBool,
    ) -> io::Result<u64> {
        Err(unavailable("the kernel copy"))
    }
}

/// The endpoint rules without `connect`: vault locators are refused, and two
/// sides of the same location must not be equal or nested.
pub struct PlainEndpointRules;

impl SyncEndpointRules for PlainEndpointRules {
    fn validate(&self, source: &str, target: &str) -> Result<(), String> {
        for side in [source, target] {
            let scheme = side
                .trim_start()
                .split_once("://")
                .map(|(scheme, _)| scheme);
            if scheme.is_some_and(|scheme| {
                scheme.eq_ignore_ascii_case("vault") || scheme.eq_ignore_ascii_case("vault-store")
            }) {
                return Err("Ein Tresor ist keine Seite eines allgemeinen Abgleichs.".into());
            }
        }
        let source = location_paths::path_key(source);
        let target = location_paths::path_key(target);
        if location_paths::paths_overlap(&source, &target) {
            return Err("Quelle und Ziel müssen verschieden und dürfen nicht ineinander verschachtelt sein (distinct, non-nested endpoints).".into());
        }
        Ok(())
    }
}
