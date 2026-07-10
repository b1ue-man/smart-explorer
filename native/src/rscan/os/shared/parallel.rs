use super::walk_state::WalkState;
use crate::vfs::BackendHandle;
use rayon::prelude::*;
use std::sync::atomic::Ordering;

/// List each breadth level concurrently, then validate, budget, and emit the
/// returned entries deterministically on the driver thread.
pub(super) fn run(backend: BackendHandle, root: String, mut state: WalkState) {
    let parallelism = backend.parallelism().clamp(2, 16);
    let pool = match rayon::ThreadPoolBuilder::new()
        .num_threads(parallelism)
        .build()
    {
        Ok(pool) => pool,
        Err(_) => {
            serial_fallback(backend, root, state);
            return;
        }
    };

    let mut frontier = vec![(root, 1u32)];
    while !frontier.is_empty() && !state.stopped() {
        let mut next = Vec::new();
        for chunk in frontier.chunks(parallelism) {
            if state.stopped() {
                break;
            }
            let cancel = state.cancel_handle();
            let listed: Vec<_> = pool.install(|| {
                chunk
                    .par_iter()
                    .map(|(directory, depth)| {
                        if cancel.load(Ordering::Relaxed) {
                            return (directory.clone(), *depth, Ok(Vec::new()));
                        }
                        (
                            directory.clone(),
                            *depth,
                            backend
                                .list_dir(directory)
                                .map_err(|error| error.to_string()),
                        )
                    })
                    .collect()
            });
            for (directory, depth, result) in listed {
                if state.stopped() {
                    break;
                }
                match result {
                    Ok(entries) => {
                        if !state.process_listing(&directory, depth, entries, true, &mut next) {
                            break;
                        }
                    }
                    Err(error) => state.listing_failed(&directory, error),
                }
                if !state.maybe_progress(&directory) {
                    break;
                }
            }
        }
        frontier = next;
    }
    state.finish();
}

fn serial_fallback(backend: BackendHandle, root: String, mut state: WalkState) {
    let mut frontier = vec![(root, 1u32)];
    while let Some((directory, depth)) = frontier.pop() {
        if state.stopped() {
            break;
        }
        match backend.list_dir(&directory) {
            Ok(entries) => {
                if !state.process_listing(&directory, depth, entries, true, &mut frontier) {
                    break;
                }
            }
            Err(error) => state.listing_failed(&directory, error),
        }
        if !state.maybe_progress(&directory) {
            break;
        }
    }
    state.finish();
}
