//! A job's live log for the Android sync screen (`sync.log`): complete lines
//! from an offset, so the screen polls only what is new.
use super::args::{bool_arg, io_error, opt_i64, str_arg};
use super::sync_jobs::find_job;
use crate::mobile::ApiError;
use serde_json::{json, Value};

/// `{id, from?}` → `{text, next, size, restarted, verbose}`; without `from`
/// the last page of the log.
pub(super) fn read(args: &Value) -> Result<Value, ApiError> {
    let job = find_job(str_arg(args, "id")?)?;
    let from = opt_i64(args, "from").and_then(|offset| u64::try_from(offset).ok());
    let chunk = crate::bisync::read_job_log(&job.id, from)
        .map_err(|error| io_error("Sync-Protokoll lesen", error))?;
    Ok(json!({
        "text": chunk.text,
        "next": chunk.next,
        "size": chunk.size,
        "restarted": chunk.restarted,
        "verbose": crate::bisync::job_log_verbose(&job.id),
    }))
}

/// `{id, verbose}`: log unchanged entries one by one from the next run on.
pub(super) fn set_verbose(args: &Value) -> Result<Value, ApiError> {
    let job = find_job(str_arg(args, "id")?)?;
    let verbose = bool_arg(args, "verbose")?;
    crate::bisync::set_job_log_verbose(&job.id, verbose)
        .map_err(|error| io_error("Protokoll-Einstellung speichern", error))?;
    Ok(json!({ "verbose": verbose }))
}
