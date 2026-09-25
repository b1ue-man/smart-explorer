//! Domain methods of the Android facade (api.md §4.6–§4.10, §5): sync jobs
//! and the background worker, connections and Google Drive, Share, storage
//! analysis and duplicates, app updates. Each handler reuses the desktop
//! core function; the core facade routes every method it does not own here.
use serde_json::Value;

use crate::mobile::{ApiError, Runtime};

#[path = "analyze.rs"]
mod analyze;
#[path = "args.rs"]
mod args;
#[path = "background.rs"]
mod background;
#[path = "connections.rs"]
mod connections;
#[path = "gdrive.rs"]
mod gdrive;
#[path = "host_keys.rs"]
mod host_keys;
#[path = "job_json.rs"]
mod job_json;
#[path = "locations.rs"]
mod locations;
#[path = "share_peers.rs"]
mod share_peers;
#[path = "share_requests.rs"]
mod share_requests;
#[path = "share_settings.rs"]
mod share_settings;
#[path = "share_state.rs"]
mod share_state;
#[path = "share_status.rs"]
mod share_status;
#[path = "sync_conflicts.rs"]
mod sync_conflicts;
#[path = "sync_jobs.rs"]
mod sync_jobs;
#[path = "sync_merge.rs"]
mod sync_merge;
#[path = "sync_run.rs"]
mod sync_run;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[path = "update.rs"]
mod update;

/// Handles a domain method; `None` = not a domain method.
pub(crate) fn dispatch(
    rt: &Runtime,
    method: &str,
    args: &Value,
) -> Option<Result<Value, ApiError>> {
    let (domain, _) = method.split_once('.')?;
    match domain {
        "sync" => sync_method(rt, method, args),
        "bg" => background_method(rt, method, args),
        "conn" | "gdrive" => connection_method(rt, method, args),
        "share" => share_method(rt, method, args),
        "analyze" | "reclaim" => analysis_method(rt, method, args),
        "update" => update_method(rt, method),
        _ => None,
    }
}

/// Starts the Share poller (it idles until the embedded worker runs).
pub(crate) fn on_init(rt: &'static Runtime) {
    share_state::start_poller(rt);
}

/// Host visibility (`sys.hostState.foreground`): the Share poller drains
/// every 5 s in the foreground and every 60 s in the background.
pub(crate) fn set_foreground(foreground: bool) {
    share_state::set_foreground(foreground);
}

fn sync_method(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "sync.options" => sync_jobs::options(),
        "sync.jobs" => sync_jobs::jobs(),
        "sync.validate" => sync_jobs::validate(args),
        "sync.save" => sync_jobs::save(rt, args),
        "sync.delete" => sync_jobs::delete(rt, args),
        "sync.setEnabled" => sync_jobs::set_enabled(rt, args),
        "sync.run" => sync_run::run(rt, args),
        "sync.mirror" => sync_run::mirror(rt, args),
        "sync.conflicts" => sync_conflicts::conflicts(args),
        "sync.checkConflicts" => sync_conflicts::check(rt, args),
        "sync.resolve" => sync_conflicts::resolve(rt, args),
        "sync.skip" => sync_conflicts::skip(args),
        "sync.finishConflicts" => sync_conflicts::finish(args),
        "sync.mergeRows" => sync_merge::rows(args),
        "sync.mergeApply" => sync_merge::apply(rt, args),
        "sync.mergeKeepBoth" => sync_merge::keep_both(rt, args),
        _ => return None,
    })
}

fn background_method(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "bg.ensureDaemon" => background::ensure_daemon(),
        "bg.status" => background::status(),
        "bg.setSyncEnabled" => background::set_sync_enabled(rt, args),
        "bg.pause" => background::pause(args),
        "bg.resume" => background::resume(),
        "bg.setAutopause" => background::set_autopause(args),
        "bg.log" => background::log(args),
        "bg.catchUp" => background::catch_up(rt),
        _ => return None,
    })
}

fn connection_method(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "conn.list" => connections::list(),
        "conn.test" => connections::test(args),
        "conn.save" => connections::save(rt, args),
        "conn.delete" => connections::delete(rt, args),
        "conn.forgetHostKey" => host_keys::forget(args),
        "gdrive.status" => gdrive::status(),
        "gdrive.configure" => gdrive::configure(args),
        "gdrive.signIn" => gdrive::sign_in(rt),
        "gdrive.signOut" => gdrive::sign_out(rt),
        _ => return None,
    })
}

fn share_method(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "share.status" => share_state::status(),
        "share.watch" => share_settings::watch(args),
        "share.setServer" => share_settings::set_server(rt, args),
        "share.setOnline" => share_settings::set_online(args),
        "share.setName" => share_settings::set_name(rt, args),
        "share.discoverable" => share_settings::discoverable(args),
        "share.stopDiscoverable" => share_settings::stop_discoverable(args),
        "share.discover" => share_settings::discover(),
        "share.connect" => share_settings::connect(args),
        "share.cancelConnect" => share_settings::cancel_connect(args),
        "share.addDirect" => share_peers::add_direct(rt, args),
        "share.removeDevice" => share_peers::remove_device(rt, args),
        "share.readmit" => share_peers::readmit(rt, args),
        "share.createRoom" => share_peers::create_room(rt, args),
        "share.joinRoom" => share_peers::join_room(rt, args),
        "share.roomCode" => share_peers::room_code(args),
        "share.leaveRoom" => share_peers::leave_room(rt, args),
        "share.removeRoom" => share_peers::remove_room(rt, args),
        "share.addExport" => share_peers::add_export(rt, args),
        "share.removeExport" => share_peers::remove_export(rt, args),
        "share.requestAccess" => share_requests::request_access(rt, args),
        "share.decide" => share_requests::decide(rt, args),
        "share.retry" => share_requests::retry(rt, args),
        "share.deleteRequest" => share_requests::delete_request(rt, args),
        "share.exec" => share_requests::exec(rt, args),
        _ => return None,
    })
}

fn analysis_method(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "analyze.start" => analyze::start_analysis(rt, args),
        "analyze.node" => analyze::node(args),
        "analyze.issues" => analyze::issues(args),
        "reclaim.start" => analyze::start_reclaim(rt, args),
        "reclaim.groups" => analyze::groups(args),
        _ => return None,
    })
}

fn update_method(rt: &Runtime, method: &str) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "update.check" => update::check(rt),
        "update.download" => update::download(rt),
        _ => return None,
    })
}
