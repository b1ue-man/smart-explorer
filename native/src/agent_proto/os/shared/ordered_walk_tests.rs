use super::*;
use std::collections::HashMap;
use std::sync::Arc;

struct Collect {
    pending: Vec<String>,
    sent: Arc<Mutex<Vec<String>>>,
}

impl Output for Collect {
    fn flush(&mut self) -> io::Result<()> {
        self.sent
            .lock()
            .unwrap()
            .extend(std::mem::take(&mut self.pending));
        Ok(())
    }
}

/// A synthetic tree: every folder above depth 4 has three subfolders and
/// two files; the folder entry is produced by its parent.
fn walk(threads: usize) -> Vec<String> {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let cancel = AtomicBool::new(false);
    run(
        vec![(String::new(), 0usize)],
        threads,
        &cancel,
        || Collect {
            pending: Vec::new(),
            sent: sent.clone(),
        },
        |(rel, depth): (String, usize), local, out: &mut Collect| {
            if depth >= 4 {
                return Ok(());
            }
            for index in 0..5 {
                let child = if rel.is_empty() {
                    format!("{index}")
                } else {
                    format!("{rel}/{index}")
                };
                out.pending.push(child.clone());
                if index < 3 {
                    local.push((child, depth + 1));
                }
            }
            Ok(())
        },
    )
    .unwrap();
    let sent = sent.lock().unwrap().clone();
    sent
}

#[test]
fn abgleich_task_ordered_walk_sends_every_folder_before_its_entries() {
    for threads in [1, 2, 8, 32] {
        let sent = walk(threads);
        // 3 + 9 + 27 + 81 folders produced 5 entries each.
        assert_eq!(sent.len(), 5 * (1 + 3 + 9 + 27), "{threads} threads");
        let positions: HashMap<&str, usize> = sent
            .iter()
            .enumerate()
            .map(|(index, rel)| (rel.as_str(), index))
            .collect();
        assert_eq!(positions.len(), sent.len(), "no entry twice");
        for (index, rel) in sent.iter().enumerate() {
            if let Some((parent, _)) = rel.rsplit_once('/') {
                assert!(positions[parent] < index, "{parent} after {rel}");
            }
        }
    }
}

#[test]
fn abgleich_task_ordered_walk_stops_at_the_first_error_and_on_cancel() {
    let cancel = AtomicBool::new(false);
    let error = run(
        vec![0u32],
        4,
        &cancel,
        || Collect {
            pending: Vec::new(),
            sent: Arc::new(Mutex::new(Vec::new())),
        },
        |job, local, _out: &mut Collect| {
            if job == 7 {
                return Err(io::Error::other("folder seven"));
            }
            if job < 7 {
                local.push(job + 1);
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "folder seven");

    let cancel = AtomicBool::new(true);
    let error = run(
        vec![0u32],
        4,
        &cancel,
        || Collect {
            pending: Vec::new(),
            sent: Arc::new(Mutex::new(Vec::new())),
        },
        |_, _, _: &mut Collect| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn abgleich_task_walk_threads_follow_the_host() {
    let expected = std::thread::available_parallelism().map_or(1, |n| n.get());
    assert_eq!(walk_threads(), expected);
}
