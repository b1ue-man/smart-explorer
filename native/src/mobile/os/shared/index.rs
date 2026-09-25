//! `index.*` (api.md §4.4): the desktop folder index over the storage
//! volumes, persisted in `<data>/folder_index.txt`, with fuzzy search.
use super::args::{str_arg, u64_or};
use super::error::ApiError;
use super::runtime::{lock, Runtime, TaskCtx};
use crate::folder_index::{FolderIndex, IndexMsg};
use crossbeam_channel::RecvTimeoutError;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_SEARCH_RESULTS: u64 = 100;

#[derive(Default)]
pub(crate) struct IndexSlot {
    state: Mutex<IndexState>,
}

#[derive(Default)]
struct IndexState {
    index: Option<Arc<FolderIndex>>,
    /// The persisted index was looked for (loaded or absent).
    loaded: bool,
    /// Task id of a running build.
    building: Option<String>,
}

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "index.status" => Ok(status(rt)),
        "index.build" => build(rt),
        "index.search" => search(rt, args),
        _ => return None,
    })
}

fn persist_path(rt: &Runtime) -> PathBuf {
    rt.config().data_dir().join("folder_index.txt")
}

/// The current index, loading the persisted one on first use.
fn current(rt: &Runtime) -> Option<Arc<FolderIndex>> {
    let mut state = lock(&rt.inner.index.state);
    if !state.loaded {
        state.loaded = true;
        match FolderIndex::load(&persist_path(rt)) {
            Ok(index) => state.index = Some(Arc::new(index)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => rt.record_error("Ordnerindex laden", &error.to_string()),
        }
    }
    state.index.clone()
}

fn status(rt: &Runtime) -> Value {
    let index = current(rt);
    let building = lock(&rt.inner.index.state).building.is_some();
    let state = if building {
        "building"
    } else if index.is_some() {
        "ready"
    } else {
        "none"
    };
    json!({ "state": state, "count": index.map_or(0, |index| index.len()) })
}

fn build(rt: &Runtime) -> Result<Value, ApiError> {
    let mut state = lock(&rt.inner.index.state);
    if let Some(id) = &state.building {
        return Ok(json!({ "taskId": id }));
    }
    let roots: Vec<PathBuf> = rt
        .volumes()
        .iter()
        .map(|volume| PathBuf::from(&volume.path))
        .filter(|path| path.is_dir())
        .collect();
    if roots.is_empty() {
        return Err(ApiError::unsupported(
            "Kein lesbarer Speicherort für den Ordnerindex",
        ));
    }
    let runtime = rt.clone();
    let id = rt.spawn_task("index", "Ordnerindex aufbauen".to_string(), move |ctx| {
        let result = run_build(&runtime, ctx, roots);
        lock(&runtime.inner.index.state).building = None;
        result
    });
    state.building = Some(id.clone());
    Ok(json!({ "taskId": id }))
}

fn run_build(rt: &Runtime, ctx: &TaskCtx, roots: Vec<PathBuf>) -> Result<Value, ApiError> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = ctx.cancel_flag();
    FolderIndex::build_async(roots, persist_path(rt), tx, cancel)
        .map_err(|error| ApiError::from(error).context("Ordnerindex starten"))?;
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(IndexMsg::Progress { count, .. }) => {
                ctx.progress(0, 0, count, 0);
                ctx.message(&format!("{count} Ordner indiziert"));
            }
            Ok(IndexMsg::Complete(index)) => {
                let count = index.len();
                let mut state = lock(&rt.inner.index.state);
                state.index = Some(Arc::new(index));
                state.loaded = true;
                ctx.message(&format!("{count} Ordner indiziert"));
                return Ok(json!({ "count": count }));
            }
            Ok(IndexMsg::Canceled) => return Err(ApiError::canceled()),
            Ok(IndexMsg::Failed(error)) => return Err(ApiError::internal(error)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(ApiError::internal("Ordnerindex endete ohne Ergebnis"))
            }
        }
    }
}

fn search(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let query = str_arg(args, "query")?.trim().to_string();
    let limit = u64_or(args, "limit", 30).clamp(1, MAX_SEARCH_RESULTS) as usize;
    let Some(index) = current(rt) else {
        return Ok(json!([]));
    };
    if query.is_empty() {
        return Ok(json!([]));
    }
    let scored = index.search_scored(&query, limit.saturating_mul(3));
    let ranked = crate::folder_index::stat_and_rank(scored, limit);
    Ok(Value::Array(
        ranked
            .into_iter()
            .map(|(path, score)| {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                json!({ "name": name, "path": path, "location": path, "score": score })
            })
            .collect(),
    ))
}
