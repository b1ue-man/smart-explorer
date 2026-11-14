//! Ordered stop of the background worker. Every step is logged, so a stop that
//! hangs shows where it stopped in the worker log. Included by `run_loop.rs`.
use super::super::ipc::ShareHost;
use super::super::ipc_listener::IpcListener;
use super::super::job_supervisor::JobSupervisor;
use super::super::login_shutdown;
use super::super::state::{clear_heartbeat, log};

pub(super) fn stop_daemon(
    supervisor: &mut JobSupervisor,
    share_host: &ShareHost,
    listener: &IpcListener,
) {
    log("daemon stopping (stop requested or unreadable stop control)");
    // No new IPC client is admitted while the worker winds down.
    listener.shutdown();
    login_shutdown::drain();
    share_host.shutdown_lan();
    // Each step is logged, so a stop that hangs shows where in the worker log.
    log("daemon stop: IPC listener closed, LAN presence withdrawn");
    share_host.stop_mounts();
    log("daemon stop: drives ended");
    for error in supervisor.cancel_and_join() {
        log(&error);
    }
    log("daemon stop: sync jobs ended");
    crate::vault::lock_all_for_exit();
    log("daemon stop: vaults locked");
    // E29 (M11): a se-managed Remote Desktop sharing is switched off with the worker.
    crate::rdp::host_setup::on_worker_stop();
    log("daemon stop: remote desktop sharing switched off");
    clear_heartbeat();
}
