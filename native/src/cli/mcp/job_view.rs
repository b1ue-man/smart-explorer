//! What a job-starting call or `job_status` returns: state, progress and one
//! page of every output stream within the answer budget, plus the offsets to
//! continue. Also the `job_input` and `job_cancel` tools.
use super::args::Args;
use super::jobs::Job;
use super::response;
use super::session::Session;
use super::spool::text_page;
use super::wait::{wait_for, Waited};
use rmcp::model::CallToolResult;
use rmcp::service::RequestContext;
use rmcp::RoleServer;
use serde_json::{json, Value};
use std::sync::Arc;

/// Answer of a call that started `job` and waited for it. A call cancelled by
/// the client cancels the job it started: the client never learned its id.
pub(super) async fn started_job_answer(
    job: &Arc<Job>,
    args: &Args<'_>,
    context: &RequestContext<RoleServer>,
) -> Result<CallToolResult, String> {
    let wait = super::wait::wait_duration(args.opt_u64("wait_seconds")?);
    let budget = response::page_budget(args.opt_u64("max_bytes")?);
    if wait_for(job, wait, context).await == Waited::ClientCancelled {
        job.cancel();
        return Ok(response::error(
            "the call was cancelled; its job was cancelled too",
        ));
    }
    Ok(response::ok(job_value(job, &Offsets::default(), budget)?))
}

/// `job_status`: one job with output pages, or every job of the session.
pub(super) async fn status_tool(
    session: &Session,
    args: &Args<'_>,
    context: &RequestContext<RoleServer>,
) -> Result<CallToolResult, String> {
    let Some(id) = args
        .opt_string("job_id")?
        .filter(|id| !id.trim().is_empty())
    else {
        let jobs = session
            .jobs
            .list()
            .iter()
            .map(|job| job.summary_value())
            .collect::<Vec<_>>();
        return Ok(response::ok(json!({ "jobs": jobs })));
    };
    let job = find(session, &id)?;
    let wait = std::time::Duration::from_secs(args.opt_u64("wait_seconds")?.unwrap_or(0));
    if !wait.is_zero() && wait_for(&job, wait, context).await == Waited::ClientCancelled {
        return Ok(response::error(
            "the call was cancelled; the job keeps running",
        ));
    }
    let offsets = Offsets {
        stdout: args.opt_u64("stdout_offset")?.unwrap_or(0),
        stderr: args.opt_u64("stderr_offset")?.unwrap_or(0),
        hits: args.opt_u64("hits_offset")?.unwrap_or(0),
    };
    let budget = response::page_budget(args.opt_u64("max_bytes")?);
    Ok(response::ok(job_value(&job, &offsets, budget)?))
}

pub(super) fn input_tool(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let job = find(session, &args.string("job_id")?)?;
    let control = job
        .control()
        .ok_or_else(|| format!("job {} is not ready for input yet", job.id))?;
    if job.end().is_some() {
        return Err(format!("job {} has already ended", job.id));
    }
    let data = args.opt_string("data")?.unwrap_or_default();
    if !data.is_empty() {
        control.stdin(data.as_bytes())?;
    }
    let eof = args.flag("eof")?;
    if eof {
        control.eof()?;
    }
    Ok(response::ok(json!({
        "job_id": job.id,
        "written_bytes": data.len(),
        "eof": eof,
    })))
}

pub(super) async fn cancel_tool(
    session: &Session,
    args: &Args<'_>,
    context: &RequestContext<RoleServer>,
) -> Result<CallToolResult, String> {
    let job = find(session, &args.string("job_id")?)?;
    if job.end().is_none() {
        job.cancel();
        // Give the worker its stop path (an SSH TERM grace ends a remote
        // process within seconds; local work stops at the next block).
        let wait = super::wait::wait_duration(args.opt_u64("wait_seconds")?);
        let _ = wait_for(&job, wait, context).await;
    }
    Ok(response::ok(job.summary_value()))
}

fn find(session: &Session, id: &str) -> Result<Arc<Job>, String> {
    session.jobs.get(id).ok_or_else(|| {
        format!("unknown job `{id}`; call job_status without job_id to list this session's jobs")
    })
}

#[derive(Default)]
pub(super) struct Offsets {
    pub(super) stdout: u64,
    pub(super) stderr: u64,
    pub(super) hits: u64,
}

impl Offsets {
    fn of(&self, stream: &str) -> u64 {
        match stream {
            "stdout" => self.stdout,
            "stderr" => self.stderr,
            _ => self.hits,
        }
    }
}

pub(super) fn job_value(job: &Job, offsets: &Offsets, budget: usize) -> Result<Value, String> {
    let mut value = job.summary_value();
    let ended = job.end().is_some();
    let names = job.stream_names().collect::<Vec<_>>();
    let share = budget / names.len().max(1);
    let object = value
        .as_object_mut()
        .ok_or_else(|| "job summary is not an object".to_string())?;
    for name in names {
        let Some(spool) = job.stream(name) else {
            continue;
        };
        let offset = offsets.of(name).min(spool.len());
        let bytes = spool
            .read(offset, share)
            .map_err(|error| format!("cannot read job output: {error}"))?;
        let page = if name == "hits" {
            hits_page(&bytes, offset)
        } else {
            let page = text_page(&bytes, offset, ended);
            json!({
                "text": page.text,
                "offset": offset,
                "next_offset": page.next_offset,
                "complete": ended && page.next_offset >= spool.len(),
                "lossy": page.lossy,
            })
        };
        object.insert(name.to_string(), page);
    }
    if !ended {
        object.insert(
            "next".into(),
            json!(format!(
                "call job_status with job_id \"{}\" (and wait_seconds) to follow it, job_cancel to stop it",
                job.id
            )),
        );
    }
    Ok(value)
}

/// Search hits are JSON lines; a page ends at the last complete line.
fn hits_page(bytes: &[u8], offset: u64) -> Value {
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let hits = bytes[..complete]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .collect::<Vec<_>>();
    json!({
        "items": hits,
        "offset": offset,
        "next_offset": offset + complete as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::{hits_page, job_value, Offsets};
    use crate::cli::mcp::jobs::{JobEnd, JobKind, JobRegistry, JobStatus};
    use serde_json::json;

    #[test]
    fn mcp_task_job_pages_split_the_budget_and_report_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let registry = JobRegistry::new(dir.path());
        let job = registry
            .create(JobKind::Exec, "exec".into(), &["stdout", "stderr"])
            .unwrap();
        job.append("stdout", &[b'a'; 3_000]).unwrap();
        job.append("stderr", b"warn").unwrap();
        let running = job_value(&job, &Offsets::default(), 2_048).unwrap();
        assert_eq!(running["status"], "running");
        assert_eq!(running["stdout"]["next_offset"], 1_024);
        assert_eq!(running["stdout"]["complete"], false);
        assert!(running["next"].as_str().unwrap().contains("j1"));
        job.finish(JobEnd {
            status: JobStatus::Completed,
            result: json!({"exit_code": 0}),
            error: None,
        });
        let offsets = Offsets {
            stdout: 1_024,
            ..Offsets::default()
        };
        let ended = job_value(&job, &offsets, 8_192).unwrap();
        assert_eq!(ended["stdout"]["next_offset"], 3_000);
        assert_eq!(ended["stdout"]["complete"], true);
        assert_eq!(ended["stderr"]["text"], "warn");
        assert_eq!(ended["exit_code"], 0);
    }

    #[test]
    fn mcp_task_hit_pages_end_at_complete_lines() {
        let bytes = b"{\"path\":\"a\"}\n{\"path\":\"b\"}\n{\"pa";
        let page = hits_page(bytes, 10);
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["next_offset"], 10 + 26);
    }
}
