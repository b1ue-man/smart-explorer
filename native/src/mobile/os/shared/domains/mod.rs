//! Domain methods of the Android facade (api.md §4.6–§4.10, §5): sync jobs
//! and the background worker, connections and Google Drive, Share, storage
//! analysis and duplicates, app updates. Each handler reuses the desktop
//! core function; the core facade routes every method it does not own here.
use serde_json::Value;

use crate::mobile::{ApiError, Runtime};

#[path = "analyze.rs"]
mod analyze;
#[path = "app_licenses.rs"]
mod app_licenses;
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
#[path = "screen_host.rs"]
mod screen_host;
// SCR-M9 (U28): this phone as screen viewer.
#[path = "screen_view.rs"]
mod screen_view;
#[path = "login_view.rs"]
mod login_view;
// DRV-M8: the documents provider (`docs.*`) and its proxy descriptors.
#[path = "docs.rs"]
mod docs;
#[path = "docs_engines.rs"]
mod docs_engines;
#[path = "docs_access.rs"]
mod docs_access;
#[path = "docs_ids.rs"]
mod docs_ids;
#[path = "docs_io.rs"]
mod docs_io;
#[path = "docs_vault.rs"]
mod docs_vault;
#[cfg(test)]
#[path = "settings_parity_tests.rs"]
mod settings_parity_tests;
#[path = "share_device_settings.rs"]
mod share_device_settings;
#[cfg(test)]
#[path = "share_device_settings_tests.rs"]
mod share_device_settings_tests;
#[path = "share_exec.rs"]
mod share_exec;
#[cfg(test)]
#[path = "share_exec_tests.rs"]
mod share_exec_tests;
#[path = "share_peers.rs"]
mod share_peers;
#[path = "share_policy.rs"]
mod share_policy;
#[path = "share_power.rs"]
mod share_power;
#[path = "share_requests.rs"]
mod share_requests;
#[path = "share_settings.rs"]
mod share_settings;
#[path = "share_state.rs"]
mod share_state;
#[path = "share_status.rs"]
mod share_status;
#[path = "route_status.rs"]
mod route_status;
#[path = "tcp.rs"]
mod tcp;
#[path = "apps.rs"]
mod apps;
#[path = "app_launch.rs"]
mod app_launch;
#[path = "rdp.rs"]
mod rdp;
#[path = "sync_conflicts.rs"]
mod sync_conflicts;
#[path = "sync_jobs.rs"]
mod sync_jobs;
#[path = "sync_log.rs"]
mod sync_log;
#[path = "sync_merge.rs"]
mod sync_merge;
#[path = "sync_run.rs"]
mod sync_run;
#[path = "sync_state_json.rs"]
mod sync_state_json;
#[path = "sync_versions.rs"]
mod sync_versions;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[path = "update.rs"]
mod update;
// VLT-M11 (U31): vaults on Android.
#[path = "vault_compare.rs"]
mod vault_compare;
#[path = "vault_keys.rs"]
mod vault_keys;
#[path = "vaults.rs"]
mod vaults;
#[path = "vault_open_policy.rs"]
mod vault_open_policy;

// SCR-M8: the binary calls of the JNI bridge (`screen_jni.rs`).
pub use docs_io::{doc_fsync, doc_open, doc_read, doc_release, doc_size, doc_write};
pub use screen_host::{screen_control, screen_push};
pub use screen_view::screen_pull;
pub use login_view::{login_pull, login_close, login_release};
// U31 S2: the label of a vault for task titles outside this module.
pub(crate) use vaults::label_of as vault_label;

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
        "tcp" => tcp::handle(method, args),
        "login" => login_view::handle(method, args),
        "apps" => app_launch::handle(method, args).or_else(|| apps::handle(method, args)),
        "rdp" => rdp::handle(method, args, |target| crate::daemon::apps_rdp_probe(target, Some(std::time::Duration::from_secs(30)), || {})),
        "analyze" | "reclaim" => analysis_method(rt, method, args),
        "update" => update_method(rt, method),
        "app" => app_licenses::handle(method),
        "screen" => screen_host::handle(method, args).or_else(|| screen_view::handle(method, args)),
        "docs" => docs::handle(rt, method, args),
        "vault" => vaults::handle(rt, method, args),
        _ => None,
    }
}

/// Starts the Share poller (it idles until the embedded worker runs).
pub(crate) fn on_init(rt: &'static Runtime) {
    share_state::start_poller(rt);
    vaults::install_events(rt);
}

/// Before the embedded worker starts: Share CPU-hold requests become `wake`
/// events.
pub(crate) fn install_activity_hook() {
    background::install_hooks();
    share_power::install_activity_hook();
}

/// Host visibility (`sys.hostState.foreground`): the Share poller drains
/// every 5 s in the foreground and every 60 s in the background, and a
/// hidden app puts the Share client into its low-power mode.
pub(crate) fn set_foreground(foreground: bool) {
    share_state::set_foreground(foreground);
    share_power::apply_visibility(foreground);
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
        "sync.confirmBlock" => sync_jobs::confirm_block(rt, args),
        "sync.versions" => sync_versions::list(rt, args),
        "sync.log" => sync_log::read(args),
        "sync.setLogVerbose" => sync_log::set_verbose(args),
        "sync.restoreVersion" => sync_versions::restore(rt, args),
        "sync.mirror" => sync_run::mirror(rt, args),
        "sync.conflicts" => sync_conflicts::conflicts(args),
        "sync.checkConflicts" => sync_conflicts::check(rt, args),
        "sync.resolve" => sync_conflicts::resolve(rt, args),
        "sync.skip" => sync_conflicts::skip(args),
        "sync.finishConflicts" => sync_conflicts::finish(args),
        "sync.mergeRows" => sync_merge::rows(args),
        "sync.mergeApply" => sync_merge::apply(rt, args),
        "sync.mergeKeepBoth" => sync_merge::keep_both(rt, args),
        "sync.mergeRetry" => sync_merge::retry(rt, args),
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
        "bg.setCadence" => background::set_cadence(args),
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
        "conn.trustHostKey" => host_keys::trust(args),
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
        "share.routeStatus" => route_status::status(args),
        "share.wake" => share_power::wake(args),
        "share.watch" => share_settings::watch(args),
        "share.setServer" => share_settings::set_server(rt, args),
        "share.serverInfo" => share_settings::server_info(),
        "share.suggestPin" => share_settings::suggest_pin(),
        "share.unconfirmedPairings" => share_settings::unconfirmed_pairings(),
        "share.resolvePairing" => share_settings::resolve_pairing(rt, args),
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
        "share.setExportAccess" => share_policy::set_export(rt, args),
        "share.connections" => share_policy::connections(args),
        "share.setConnectionExport" => share_policy::set_connection(rt, args),
        "share.setContactWrite" => share_policy::set_contact_write(rt, args),
        "share.allowGrantAgain" => share_policy::allow_grant_again(rt, args),
        "share.withdrawGrant" => share_policy::withdraw_grant(rt, args),
        "share.setRoomMember" => share_policy::set_room_member(rt, args),
        "share.setShareBack" => share_policy::set_share_back(rt, args),
        "share.setRoomPolicy" => share_policy::set_room_policy(rt, args),
        "share.policy" => share_policy::policy(),
        "share.setPolicy" => share_policy::set_policy(args),
        "share.requestAccess" => share_requests::request_access(rt, args),
        "share.decide" => share_requests::decide(rt, args),
        "share.retry" => share_requests::retry(rt, args),
        "share.deleteRequest" => share_requests::delete_request(rt, args),
        "share.exec" => share_requests::exec(rt, args),
        "share.setExec" => share_exec::set_exec(args),
        "share.execJobs" => share_exec::exec_jobs(),
        "share.cancelExecJob" => share_exec::cancel_exec_job(args),
        "share.execLog" => share_exec::exec_log(args),
        "share.setAccess" => share_exec::access::set_access(args),
        "share.deviceRights" => share_exec::access::device_rights(args),
        "share.accessApprovals" => share_exec::access::approvals(),
        "share.decideAccessApproval" => share_exec::access::decide(args),
        "share.reportApprovalUndeliverable" => share_exec::access::report_undeliverable(args),
        "share.setContactOptions" => share_device_settings::set_contact_options(rt, args),
        "share.resetContactTrust" => share_device_settings::reset_contact_trust(rt, args),
        "share.renameRoom" => share_device_settings::rename_room(rt, args),
        "share.rejoinRoom" => share_device_settings::rejoin_room(rt, args),
        "share.resetMemberTrust" => share_device_settings::reset_member_trust(rt, args),
        "share.moveExport" => share_device_settings::move_export(rt, args),
        "share.setDirectOnline" => share_device_settings::set_direct_online(args),
        "share.setLanPresence" => share_device_settings::set_lan_presence(rt, args),
        "share.renewCode" => share_device_settings::renew_code(rt),
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
        "reclaim.summary" => analyze::summary(args),
        "reclaim.recycle" => analyze::start_recycle(rt, args),
        "reclaim.recycleResult" => analyze::recycle_result(args),
        "analyze.release" | "reclaim.release" => analyze::release(args),
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
