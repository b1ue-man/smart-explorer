//! Android core facade: JSON commands, tasks and events behind the JNI
//! bridge (`native/android-bridge`). The contract is
//! `docs/superpowers/plans/2026-09-25-android-apk/api.md`; `init`, `call` and
//! `poll_events` answer `{"ok": …}` or `{"err": {"kind", "message"}}`.
//!
//! Compiled for Android and for the Linux unit tests only. This module owns
//! the protocol, the task register, the event queue, the backend pool and the
//! file methods (`sys.* task.* loc.* fs.* scan.* index.* trash.*`); every
//! other method goes to `domains::dispatch`.

#[path = "core/args.rs"]
mod args;
#[path = "core/config.rs"]
mod config;
#[path = "core/crumbs.rs"]
mod crumbs;
#[path = "os/shared/delete.rs"]
mod delete;
#[path = "os/shared/dispatch.rs"]
mod dispatch;
#[path = "os/shared/domains/mod.rs"]
mod domains;
#[path = "os/shared/drive.rs"]
mod drive;
#[path = "os/shared/edits.rs"]
mod edits;
#[path = "os/shared/edits_store.rs"]
mod edits_store;
#[path = "core/entry.rs"]
mod entry;
#[path = "core/error.rs"]
mod error;
#[path = "core/events.rs"]
mod events;
#[path = "os/shared/fs_edit.rs"]
mod fs_edit;
#[path = "os/shared/fs_list.rs"]
mod fs_list;
#[path = "os/shared/import.rs"]
mod import;
#[path = "os/shared/index.rs"]
mod index;
#[path = "os/shared/init.rs"]
mod init;
#[path = "core/location.rs"]
mod location;
#[path = "os/shared/places.rs"]
mod places;
#[path = "os/shared/pool.rs"]
mod pool;
#[path = "os/shared/runtime.rs"]
mod runtime;
#[path = "os/shared/scan.rs"]
mod scan;
#[path = "core/scanview.rs"]
mod scanview;
#[path = "core/slots.rs"]
mod slots;
#[path = "os/shared/store.rs"]
mod store;
#[path = "os/shared/sys.rs"]
mod sys;
#[path = "core/tasks.rs"]
mod tasks;
#[path = "os/shared/transfer.rs"]
mod transfer;
#[path = "os/shared/trash.rs"]
mod trash;
#[path = "os/unix_fd.rs"]
mod unix_fd;

#[cfg(test)]
#[path = "core/tests.rs"]
mod core_tests;
#[cfg(test)]
#[path = "os/shared/tests.rs"]
mod facade_tests;

#[allow(unused_imports)]
pub(crate) use config::HostSettings;
pub(crate) use error::ApiError;
pub(crate) use runtime::{Runtime, TaskCtx};

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

/// Longest wait of one `poll_events`.
const MAX_POLL_WAIT: Duration = Duration::from_secs(30);

fn guarded(run: impl FnOnce() -> Result<serde_json::Value, ApiError>) -> String {
    let result = catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| {
        Err(ApiError::internal(
            "Interner Fehler im Kern (Details im Absturzprotokoll)",
        ))
    });
    error::envelope(result)
}

/// First call: configures the host values and starts the runtime; later
/// calls only update the storage volumes. Never waits on network or daemon.
pub fn init(config: &str) -> String {
    guarded(|| init::init(config))
}

/// Runs one method synchronously (long work is started as a task).
pub fn call(method: &str, args: &str) -> String {
    guarded(|| {
        let rt = Runtime::get()?;
        let args: serde_json::Value = if args.trim().is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_str(args)
                .map_err(|error| ApiError::invalid(format!("Argumente: {error}")))?
        };
        dispatch::dispatch(rt, method, &args)
    })
}

/// Waits up to `timeout` for events and returns at most 256 of them.
pub fn poll_events(timeout: Duration) -> String {
    guarded(|| {
        let rt = Runtime::get()?;
        let events = rt.hub().poll(timeout.min(MAX_POLL_WAIT));
        Ok(serde_json::Value::Array(events))
    })
}
