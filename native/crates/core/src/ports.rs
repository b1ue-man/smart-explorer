//! Ports: how a lower layer of the engine (core, storage, transfer; plan
//! docs/plaene/2026-11-01-idealzustand, D13) reaches an implementation that
//! lives in a higher layer without naming the higher module.
//!
//! The lower module owns the port: a trait and one accessor written with
//! [`declare_port!`]. The higher module implements the trait on a unit struct
//! and names it once with [`provide_port!`]. The linker joins the two through
//! one external symbol; nothing is registered at run time. A program, test
//! binary or tool that links a lower module without the provider of one of
//! its ports therefore does not link (undefined symbol `smart_explorer_port_*`):
//! no entry point can forget a registration, and no lower module ever runs
//! without its implementation or with a weaker stand-in.
//!
//! Every port, its lower owner and its provider:
//!
//! | Symbol | Trait (owner) | Provider |
//! |---|---|---|
//! | `smart_explorer_port_transfer_vault` | `transfer::TransferVault` | `vault` (os/shared/transfer_port.rs) |
//! | `smart_explorer_port_transfer_worker` | `transfer::TransferWorker` | `daemon` (os/shared/transfer_job_client.rs) |
//! | `smart_explorer_port_vfs_sealed_spools` | `vfs::SealedSpools` | `vault` (os/shared/ephemeral_spool.rs) |
//! | `smart_explorer_port_vfs_sftp` | `vfs::UrlBackends` | `sftp` (core/url.rs) |
//! | `smart_explorer_port_vfs_smb` | `vfs::UrlBackends` | `smb` (core/url.rs) |
//! | `smart_explorer_port_vfs_sync_endpoints` | `vfs::SyncEndpointRules` | `connect` (core/location.rs) |
//! | `smart_explorer_port_vfs_local_server_copy` | `vfs::LocalServerCopy` | `copy` (os/shared/server_copy.rs) |
//!
//! Each symbol is declared exactly once (by its owner) and defined exactly
//! once (by its provider), both with the trait of this table: the accessor
//! and the definition must name the same trait object type, which the two
//! macros keep to one written type per side.

/// Declares the accessor of a port in its lower owner module:
///
/// ```ignore
/// crate::declare_port! {
///     /// What the accessor returns.
///     pub fn vault() -> dyn TransferVault = smart_explorer_port_transfer_vault;
/// }
/// ```
///
/// The accessor returns the one implementation the higher layer provides
/// with [`provide_port!`] under the same symbol and trait object type.
#[macro_export]
macro_rules! declare_port {
    ($(#[$meta:meta])* $vis:vis fn $accessor:ident() -> $port:ty = $symbol:ident;) => {
        $(#[$meta])*
        $vis fn $accessor() -> &'static $port {
            extern "Rust" {
                fn $symbol() -> &'static $port;
            }
            // SAFETY: `provide_port!` defines `$symbol` exactly once, as a
            // Rust-ABI function without arguments that returns
            // `&'static $port` (table in `crate::ports`); it has no
            // preconditions and touches no shared state.
            unsafe { $symbol() }
        }
    };
}

/// Provides the implementation of a port in its higher module:
///
/// ```ignore
/// struct VaultTransfers;
/// impl crate::transfer::TransferVault for VaultTransfers { /* ... */ }
/// crate::provide_port!(smart_explorer_port_transfer_vault: dyn crate::transfer::TransferVault = VaultTransfers);
/// ```
///
/// `$implementation` is a unit struct without fields; one static instance
/// serves every caller.
#[macro_export]
macro_rules! provide_port {
    ($symbol:ident: $port:ty = $implementation:ident) => {
        #[doc(hidden)]
        #[no_mangle]
        pub extern "Rust" fn $symbol() -> &'static $port {
            static IMPLEMENTATION: $implementation = $implementation;
            &IMPLEMENTATION
        }
    };
}
