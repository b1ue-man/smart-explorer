//! What changes without a share event (E21, step 2 (e) and (f)): the
//! configured Share server's relays, a restart of the login broker (its
//! policy may still hold a grant revoked while it was down) and a sync that
//! did not reach the broker. Each change starts one `LoginTrigger::Watch`
//! round, which only syncs a running service and never prompts.
use super::host_status;
use crate::login::core::{gateway_types::GatewayNetwork, self_plan::LoginTrigger};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static STARTED: AtomicBool = AtomicBool::new(false);
/// The last sync did not reach the broker; every server round asks again.
static UNREACHABLE: AtomicBool = AtomicBool::new(false);
/// Broker evidence is a small public record: one look every five seconds.
const TICK: Duration = Duration::from_secs(5);
/// The server configuration and an unreachable broker: every 30 seconds.
const SERVER_TICKS: u32 = 6;

/// Records `now` in `seen`; `true` when an earlier look saw something else.
/// The first look only records: the start trigger syncs that state already.
pub fn note<T: PartialEq>(seen: &mut Option<T>, now: T) -> bool {
    let changed = seen.as_ref().is_some_and(|before| *before != now);
    *seen = Some(now);
    changed
}

/// `true` after a sync that did not reach the broker, `false` after one
/// that did (a refusal is no reason to ask again).
pub fn note_unreachable(unreachable: bool) {
    UNREACHABLE.store(unreachable, Ordering::Release);
}

/// The process instance of the running broker, if it published one.
fn broker_instance() -> Option<String> {
    host_status::activity()
        .evidence
        .map(|evidence| evidence.process_instance)
}

/// The relays `sync` would send now; `None` while the server is unreadable.
fn server_network() -> Option<GatewayNetwork> {
    crate::daemon::load_share_server()
        .ok()
        .map(|server| crate::share::login_transport::configured_server_network(&server))
}

/// Starts the watch once per process. A platform without a privileged
/// login host (Android) never starts it, so it costs no wake-ups there.
pub fn start() {
    if host_status::activity().state == "unavailable" || STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("login-self-watch".into())
        .spawn(|| {
            let (mut broker, mut network) = (None, None);
            let mut round = 0u32;
            loop {
                let mut due = note(&mut broker, broker_instance());
                if round % SERVER_TICKS == 0 {
                    due |= note(&mut network, server_network());
                    due |= UNREACHABLE.load(Ordering::Acquire);
                }
                round = round.wrapping_add(1);
                if due {
                    super::self_setup::request(LoginTrigger::Watch);
                }
                std::thread::sleep(TICK);
            }
        });
    if spawned.is_err() {
        STARTED.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "self_watch_task_tests.rs"]
mod self_watch_task_tests;
