//! The poll for a changed system wallpaper (U34-4): gsettings, xfconf and
//! the KDE file give no common change notice, so a thread reads the source
//! of every monitor the process asked about every five seconds and compares
//! it with the value before. `changed(monitor)` tells the view once per
//! change; only a monitor whose system wallpaper was read is watched. No
//! file watch.
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use super::wallpaper_system::signature;

/// How often the source is read.
pub(crate) const POLL: Duration = Duration::from_secs(5);

struct Entry {
    monitor: String,
    last: Option<String>,
    changed: bool,
}

#[derive(Default)]
struct Watch {
    entries: Vec<Entry>,
    running: bool,
}

static WATCH: OnceLock<Mutex<Watch>> = OnceLock::new();

fn watch() -> MutexGuard<'static, Watch> {
    WATCH
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn run() {
    loop {
        let monitors: Vec<String> = watch()
            .entries
            .iter()
            .map(|entry| entry.monitor.clone())
            .collect();
        for monitor in monitors {
            // The source is read without the lock: a command may take its
            // five seconds.
            let now = signature(&monitor);
            let mut state = watch();
            if let Some(entry) = state
                .entries
                .iter_mut()
                .find(|entry| entry.monitor == monitor)
            {
                if entry.last.as_ref().is_some_and(|last| *last != now) {
                    entry.changed = true;
                }
                entry.last = Some(now);
            }
        }
        std::thread::sleep(POLL);
    }
}

/// Starts watching the wallpaper of `monitor` (the surface reads it as its
/// background); the first call starts the poll.
pub(crate) fn watch_monitor(monitor: &str) {
    let mut state = watch();
    if !state.entries.iter().any(|entry| entry.monitor == monitor) {
        state.entries.push(Entry {
            monitor: monitor.to_string(),
            last: None,
            changed: false,
        });
    }
    if !state.running {
        let spawned = std::thread::Builder::new()
            .name("se-wallpaper-watch".into())
            .spawn(run);
        state.running = spawned.is_ok();
    }
}

/// Whether the wallpaper of `monitor` changed since the last call; always
/// `false` for a monitor nobody watches (a surface with a colour or an image
/// of its own never reads the system's).
pub(crate) fn changed(monitor: &str) -> bool {
    watch()
        .entries
        .iter_mut()
        .find(|entry| entry.monitor == monitor)
        .is_some_and(|entry| std::mem::take(&mut entry.changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unwatched_monitor_never_changes() {
        assert!(!changed("never-watched"));
        {
            let mut state = watch();
            state.entries.push(Entry {
                monitor: "marked".into(),
                last: Some("a".into()),
                changed: true,
            });
        }
        assert!(changed("marked"), "the change is told once");
        assert!(!changed("marked"));
    }
}
