//! `remove_tree_v1`: the host deletes a whole folder for one streamed
//! request. The work runs on the Background blocking class (never behind
//! analyses), keeps every protection of `RemoveDir` (destructive preflight,
//! held handles, a live write check before every entry) and reports its
//! counters every 250 ms. A stopped stream or a revoked right ends it at the
//! next entry; removed entries stay removed.
use crate::share::{
    blocking::{run_for, Class},
    framing::{reply, reply_err},
    fs_access::FsAccess,
    fs_response::FsRemoveTreeMessage,
    host_stream::Stop,
    io_deadline,
    mount_lease::{run_authorized, MountLeaseAuthorization},
    session::PeerPrincipal,
    wire::FsResponse,
};
use iroh::endpoint::SendStream;
use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// What the worker reached; the stream reports it.
#[derive(Default)]
struct RemovalCounters {
    applying: AtomicBool,
    planned: AtomicU64,
    removed: AtomicU64,
}

impl RemovalCounters {
    fn progress(&self) -> FsRemoveTreeMessage {
        let (planned, removed) = self.totals();
        FsRemoveTreeMessage::Progress {
            applying: self.applying.load(Ordering::Acquire),
            planned,
            removed,
        }
    }

    fn done(&self) -> FsRemoveTreeMessage {
        let (planned, removed) = self.totals();
        FsRemoveTreeMessage::Done { planned, removed }
    }

    /// Both only grow. Non-local exports have no preflight walk and entries
    /// added during the delete were never planned, so `planned` reports at
    /// least `removed`.
    fn totals(&self) -> (u64, u64) {
        let removed = self.removed.load(Ordering::Acquire);
        (self.planned.load(Ordering::Acquire).max(removed), removed)
    }
}

pub(in crate::share) async fn serve(
    mut send: SendStream,
    path: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    // Dropping this request (stream stopped, connection gone) stops the worker.
    let _stop = Stop(cancel.clone());
    if let Err(error) = access.register_cancel(&cancel) {
        return reply_err(&mut send, error).await;
    }
    let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
    let counters = Arc::new(RemovalCounters::default());
    let worker = {
        let (cancel, counters) = (cancel.clone(), counters.clone());
        run_for(
            principal,
            Class::Background,
            "Share remove tree",
            move || {
                run_authorized(authorization.as_ref(), || {
                    remove(&access, &path, &cancel, &counters)
                })
            },
        )
    };
    tokio::pin!(worker);
    let stopped = send.stopped();
    tokio::pin!(stopped);
    let mut tick = tokio::time::interval(PROGRESS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut worker => {
                return match result {
                    Ok(()) => {
                        let message = counters.done();
                        reply(&mut send, FsResponse::RemoveTree { message }).await
                    }
                    Err(error) => reply_err(&mut send, error).await,
                };
            }
            _ = &mut stopped => {
                cancel.store(true, Ordering::Release);
                return Err(io::ErrorKind::Interrupted.into());
            }
            _ = tick.tick() => {
                let message = counters.progress();
                io_deadline::run(
                    "Share remove tree progress",
                    reply(&mut send, FsResponse::RemoveTree { message }),
                )
                .await?;
            }
        }
    }
}

/// The blocking work of one request. A revocation also sets `cancel`; the
/// lost right is then reported instead of a bare interruption.
fn remove(
    access: &FsAccess,
    path: &str,
    cancel: &AtomicBool,
    counters: &RemovalCounters,
) -> io::Result<()> {
    match remove_checked(access, path, cancel, counters) {
        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
            Err(access.check_write().err().unwrap_or(error))
        }
        other => other,
    }
}

/// Refuse export roots and anything but a plain folder, run the destructive
/// preflight while counting `planned`, then delete through the held handles
/// while counting `removed`.
fn remove_checked(
    access: &FsAccess,
    path: &str,
    cancel: &AtomicBool,
    counters: &RemovalCounters,
) -> io::Result<()> {
    if crate::share::fs::is_export_root(path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Die Wurzel einer Freigabe wird nie gelöscht",
        ));
    }
    let stop = || -> io::Result<()> {
        if cancel.load(Ordering::Acquire) {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Ok(())
        }
    };
    stop()?;
    let target = access.resolve_write(path)?;
    let current = target.backend.stat(&target.path)?;
    if current.is_symlink || !current.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Löschziel ist kein gewöhnlicher Ordner",
        ));
    }
    counters.planned.store(1, Ordering::Release); // the folder itself
    crate::share::fs::require_target_destructive_observed(&target, &|| {
        stop()?;
        counters.planned.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })?;
    counters.applying.store(true, Ordering::Release);
    crate::share::fs_delete::remove_tree_observed(
        &*target.backend,
        &target.path,
        &|| {
            stop()?;
            access.check_write()
        },
        &|| {
            counters.removed.fetch_add(1, Ordering::AcqRel);
        },
    )
}

#[cfg(test)]
#[path = "host_remove_tree_task_tests.rs"]
mod task_tests;
