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

#[path = "core_oslocked/connector.rs"]
mod connector;
#[path = "core/endpoint.rs"]
mod endpoint;
#[path = "core_oslocked/persistence.rs"]
mod persistence;
#[path = "core/types.rs"]
mod types;

pub use connector::{open_gdrive, resolve_endpoint, spawn_connect};
#[allow(unused_imports)]
pub use connector::open_saved_at;
pub use endpoint::{is_remote_url, saved_and_path};
#[allow(unused_imports)]
pub use endpoint::{gdrive_endpoint, remote_endpoint};
#[allow(unused_imports)]
pub use persistence::build_saved;
pub use types::{Connected, ConnectForm, ConnectResult, RemoteState};
