use crossbeam_channel::Sender;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::model::{FolderIndex, IndexMsg};

#[path = "persistence.rs"]
mod persistence;
#[path = "rank.rs"]
mod rank;
#[path = "walk.rs"]
mod walk;

pub use rank::stat_and_rank;

impl FolderIndex {
    /// Build and atomically persist an index on a detached worker. A terminal
    /// message is sent only after traversal and persistence have both finished.
    pub fn build_async(
        roots: Vec<PathBuf>,
        persist_path: PathBuf,
        tx: Sender<IndexMsg>,
        cancel: Arc<AtomicBool>,
    ) -> io::Result<()> {
        std::thread::Builder::new()
            .name("index-builder".into())
            .spawn(move || run_builder(roots, persist_path, tx, cancel))
            .map(|_| ())
    }
}

fn run_builder(
    roots: Vec<PathBuf>,
    persist_path: PathBuf,
    tx: Sender<IndexMsg>,
    cancel: Arc<AtomicBool>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        build_message(roots, &persist_path, &tx, &cancel)
    }));
    let message = match result {
        Err(payload) => IndexMsg::Failed(format!(
            "folder-index worker panicked: {}",
            panic_detail(payload)
        )),
        Ok(Some(message)) => message,
        Ok(None) => return,
    };

    if tx.send(message).is_err() {
        cancel.store(true, Ordering::Release);
    }
}

fn build_message(
    roots: Vec<PathBuf>,
    persist_path: &std::path::Path,
    tx: &Sender<IndexMsg>,
    cancel: &AtomicBool,
) -> Option<IndexMsg> {
    let index = match walk::build_index(roots, tx, cancel) {
        Ok(index) => index,
        Err(walk::WalkStop::Canceled) => return Some(IndexMsg::Canceled),
        Err(walk::WalkStop::ReceiverClosed) => return None,
        Err(walk::WalkStop::Failed(error)) => return Some(IndexMsg::Failed(error)),
    };
    if cancel.load(Ordering::Acquire) {
        return Some(IndexMsg::Canceled);
    }
    if tx
        .send(IndexMsg::Progress {
            count: index.len() as u64,
            current: "Index wird gespeichert…".to_string(),
        })
        .is_err()
    {
        return None;
    }
    Some(
        match index.save_cancellable(persist_path, || cancel.load(Ordering::Acquire)) {
            Ok(persistence::SaveOutcome::Saved) => IndexMsg::Complete(index),
            Ok(persistence::SaveOutcome::Canceled) => IndexMsg::Canceled,
            Err(error) => IndexMsg::Failed(format!("folder index could not be saved: {error}")),
        },
    )
}

fn panic_detail(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}
