//! `sys.*` (api.md §4.1) and `task.*` (api.md §3).
use super::args::{bool_or, opt_str, str_arg, str_list};
use super::config::{validate_volume, VolumeInfo};
use super::error::ApiError;
use super::runtime::Runtime;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};

/// The crash log is returned up to this many trailing bytes.
const MAX_CRASH_LOG_BYTES: u64 = 512 * 1024;

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "sys.hostState" => host_state(args),
        "sys.volumes" => volumes(rt, args),
        "sys.errors" => Ok(Value::Array(rt.error_log())),
        "sys.clearErrors" => {
            rt.clear_error_log();
            Ok(json!({}))
        }
        "sys.crashLog" => crash_log(rt),
        "sys.info" => Ok(json!({
            "coreVersion": env!("CARGO_PKG_VERSION"),
            "dataDir": rt.config().data_dir().to_string_lossy(),
            "cacheDir": rt.config().cache_dir.to_string_lossy(),
        })),
        "task.list" => Ok(Value::Array(rt.hub().with(|state| state.tasks.snapshots()))),
        "task.get" => task_get(rt, args),
        "task.cancel" => task_cancel(rt, args),
        "task.cancelAll" => {
            let kind = opt_str(args, "kind");
            rt.hub().with(|state| state.tasks.cancel_all(kind));
            Ok(json!({}))
        }
        "task.clear" => task_clear(rt, args),
        _ => return None,
    })
}

/// `task.clear {ids?}`: finished tasks, all or only the listed ones.
fn task_clear(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let ids = match args.get("ids") {
        None | Some(Value::Null) => None,
        Some(_) => Some(str_list(args, "ids")?),
    };
    rt.hub()
        .with(|state| state.tasks.clear_finished(ids.as_deref()));
    Ok(json!({}))
}

/// Energy saving and metered networks feed the daemon's automatic pause;
/// `deferScheduling` holds the daemon's scheduled jobs (the host's own worker
/// runs them); visibility sets the Share poller's cadence and the Share
/// client's low-power mode. `wifi`/`charging` are only used by the host's own
/// scheduling. Missing flags read as false.
fn host_state(args: &Value) -> Result<Value, ApiError> {
    let (state, foreground) = parse_host_state(args);
    crate::daemon::set_host_state(state);
    super::domains::set_foreground(foreground);
    Ok(json!({}))
}

fn parse_host_state(args: &Value) -> (crate::daemon::HostState, bool) {
    let state = crate::daemon::HostState {
        power_save: bool_or(args, "powerSave", false),
        metered: bool_or(args, "metered", false),
        defer_scheduling: bool_or(args, "deferScheduling", false),
    };
    (state, bool_or(args, "foreground", false))
}

fn volumes(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let value = args
        .get("volumes")
        .cloned()
        .ok_or_else(|| ApiError::invalid("Argument „volumes“ fehlt"))?;
    let volumes: Vec<VolumeInfo> = serde_json::from_value(value)
        .map_err(|error| ApiError::invalid(format!("Speicherorte: {error}")))?;
    for volume in &volumes {
        validate_volume(volume)?;
    }
    rt.set_volumes(volumes);
    rt.emit(json!({ "type": "volumes" }));
    Ok(json!({}))
}

fn crash_log(rt: &Runtime) -> Result<Value, ApiError> {
    let path = rt.config().data_dir().join("crash.log");
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({ "text": "" }))
        }
        Err(error) => return Err(ApiError::from(error).context("Absturzprotokoll lesen")),
    };
    let length = file.metadata()?.len();
    let start = length.saturating_sub(MAX_CRASH_LOG_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(MAX_CRASH_LOG_BYTES).read_to_end(&mut bytes)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        text.insert_str(0, "… (gekürzt)\n");
    }
    Ok(json!({ "text": text }))
}

fn task_get(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "id")?;
    rt.hub()
        .with(|state| state.tasks.get(id).map(|record| record.snapshot()))
        .ok_or_else(|| ApiError::not_found(format!("Unbekannter Vorgang: {id}")))
}

fn task_cancel(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "id")?;
    if rt.hub().with(|state| state.tasks.cancel(id)) {
        Ok(json!({}))
    } else {
        Err(ApiError::not_found(format!("Unbekannter Vorgang: {id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_host_state;
    use serde_json::json;

    #[test]
    fn android_background_task_host_state_reads_defer_scheduling() {
        let (state, foreground) = parse_host_state(&json!({
            "powerSave": true,
            "metered": false,
            "wifi": true,
            "charging": false,
            "foreground": false,
            "deferScheduling": true,
        }));
        assert!(state.power_save && !state.metered && state.defer_scheduling);
        assert!(!foreground);
        // An older host without the flag keeps scheduling as before.
        let (state, foreground) = parse_host_state(&json!({ "foreground": true }));
        assert!(!state.defer_scheduling && !state.power_save);
        assert!(foreground);
    }
}
