//! Connect orchestration: turn a `ConnectForm` (or a saved connection) into a
//! live backend, off the UI thread. Keeps app.rs thin - it only renders the
//! form and drains the result.
//!
//! Routing once connected (decided in app.rs):
//!  * SFTP / FTP / FTPS  -> a `RemoteState` backend; navigation walks it via
//!    `rscan` (remote scan path).
//!  * Network share      -> authenticated with `net::NetConnection`; the UNC path
//!    is then browsed by the LOCAL scanner (std::fs handles UNC), so no
//!    `RemoteState` - only the live `NetConnection` is kept alive.

#[path = "os/shared/cleanup.rs"]
mod cleanup;
#[path = "os/shared/connector.rs"]
mod connector;
#[path = "core/endpoint.rs"]
mod endpoint;
#[path = "core/location.rs"]
mod location;
#[path = "os/shared/location_prefs.rs"]
mod location_prefs;
#[path = "os/shared/persistence.rs"]
mod persistence;
#[path = "core/removal_scope.rs"]
mod removal_scope;
#[path = "os/shared/resolution.rs"]
mod resolution;
#[path = "core/types.rs"]
mod types;

pub use cleanup::cleanup_removed_endpoint_state;
#[allow(unused_imports)]
pub use connector::open_saved_at;
pub(crate) use connector::open_saved_at_for_mount;
pub use connector::{open_gdrive, spawn_connect};
pub(crate) use endpoint::parse_remote_url;
#[allow(unused_imports)]
pub use endpoint::{gdrive_endpoint, remote_endpoint};
pub use endpoint::{is_remote_url, saved_and_path};
pub(crate) use location::paths_overlap as location_paths_overlap;
pub(crate) use location::{local_root, validate_sync_endpoints};
pub use location_prefs::{
    favorites_path, load_dir_sort, load_favorites, save_dir_sort, save_favorites,
};
#[allow(unused_imports)]
pub use persistence::build_saved;
pub use removal_scope::{location_key, CleanupReport, MountScope, RemovedEndpointScope};
pub use resolution::resolve_endpoint;
pub use types::{ConnectForm, ConnectResult, Connected, RemoteState};

#[cfg(test)]
#[path = "os/shared/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;
#[cfg(test)]
#[path = "core/sync_paths_task_tests.rs"]
mod sync_paths_task_tests;
