//! Virtual filesystem layer - the single, standardized interface Smart Explorer
//! talks to, on top of which every storage backend is built: local disk today,
//! SFTP / FTP / network drives next, cloud later. See
//! `docs/REMOTE_LAYER_PLAN.md`.
//!
//! Design (verified):
//!  * **The trait is BLOCKING.** The whole app is synchronous (rayon +
//!    `std::thread` + crossbeam). Remote backends own a private runtime and
//!    `block_on` internally, so scanner / copy / UI never see async.
//!  * **Paths are FORWARD-SLASH strings.** The app already stores paths that
//!    way; each backend converts to its own convention at the boundary.
//!  * **Self-contained.** This module adds no edits to the hot local scan/copy
//!    loops. `LocalBackend` mirrors today's `std::fs` behavior so the remote
//!    scan/copy paths added with the SFTP/FTP backends (and any later
//!    unification) can route through ONE interface without putting a vtable in
//!    the hot local walk. The local fast path stays exactly as it is.
#![allow(dead_code)] // staged interface: wired in by the SFTP/FTP/connect steps.

#[path = "core/cache.rs"]
mod cache;
#[path = "core/core.rs"]
mod core;
#[path = "core/dispatch.rs"]
mod dispatch;
#[path = "os/shared/local.rs"]
mod local;

pub use self::cache::CachingBackend;
pub use self::core::{Backend, BackendHandle, HashHit, Scheme, SearchHit, VfsMeta, VfsResult};
#[allow(unused_imports)]
pub use self::dispatch::{backend_for, is_remote_root};
pub use self::local::LocalBackend;

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
