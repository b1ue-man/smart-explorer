//! `se screen sessions` and `se screen stop` (SCR-M2): the screen sessions
//! this device hosts, from the running background worker only (without one
//! there are none, and none is started), and their end by the host user.
use super::output;

pub(super) fn list(json: bool) -> Result<i32, String> {
    let (_, sessions) = crate::daemon::screen_sessions()?;
    let value = output::sessions_json(&sessions);
    output::print(json, &value, || output::sessions_text(&sessions));
    Ok(0)
}

pub(super) fn stop(session: Option<String>, all: bool, json: bool) -> Result<i32, String> {
    let selected = if all { None } else { session.clone() };
    let count = crate::daemon::screen_stop(selected)?;
    if let (Some(id), false, 0) = (&session, all, count) {
        return Err(format!(
            "no running screen session {id}; see `se screen sessions`"
        ));
    }
    let value = output::stop_json(count);
    output::print(json, &value, || format!("stopped\t{count}\n"));
    Ok(0)
}
