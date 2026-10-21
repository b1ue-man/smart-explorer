//! The system sensors behind the limiter and the displays: one probe thread
//! reads the machine through the platform adapter once a second and keeps the
//! latest `SystemSample`. It starts with the first request, rests after a
//! minute without one and wakes with the next. Nothing here needs a value: a
//! source the platform cannot read is an unknown value (`None`), never an
//! error, and the callers use only what is known (U44-6 point 3).
use super::platform;
pub(crate) use super::sensor_parse::SystemSample;
use super::sensor_parse::{sample_from, RawSample};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How often the machine is read.
const PROBE_INTERVAL: Duration = Duration::from_secs(1);
/// The probe thread rests after this long without a request.
const REST_AFTER: Duration = Duration::from_secs(60);

struct State {
    sample: SystemSample,
    last_request: Instant,
    running: bool,
    /// Block devices (`major:minor`) whose queue is read.
    watch: Vec<String>,
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(|| {
            Mutex::new(State {
                sample: unknown(),
                last_request: Instant::now(),
                running: false,
                watch: Vec::new(),
            })
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A sample without any reading.
fn unknown() -> SystemSample {
    sample_from(None, &RawSample::default(), 0)
}

/// The latest reading; the first call (and the first after a rest) starts
/// the probe thread and answers with what is known without a reading.
pub(crate) fn latest() -> SystemSample {
    let mut state = state();
    state.last_request = Instant::now();
    if !state.running {
        state.running = std::thread::Builder::new()
            .name("transfer-sensors".into())
            .spawn(probe)
            .is_ok();
    }
    state.sample.clone()
}

/// Queue depth of the block device holding `path`, once the probe has read
/// it twice; asking for a device makes the probe watch it from then on.
pub(crate) fn disk_queue_for(path: &str) -> Option<f64> {
    let id = platform::device_id_of(std::path::Path::new(path))?;
    let mut state = state();
    if !state.watch.contains(&id) {
        state.watch.push(id.clone());
    }
    state
        .sample
        .disk_queues
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, queue)| *queue)
}

/// Whether both paths lie on the same volume: the same drive letter, or the
/// same block device. Without evidence for either (a file system with no
/// device behind it) the answer is no.
pub(crate) fn same_volume(one: &str, other: &str) -> bool {
    match (drive_prefix(one), drive_prefix(other)) {
        (Some(a), Some(b)) => return a == b,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => {}
    }
    match (
        platform::device_id_of(std::path::Path::new(one)),
        platform::device_id_of(std::path::Path::new(other)),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// A name of the volume holding `path` for the texts: the drive letter on
/// Windows, the kernel's device name on Linux. Never a folder name.
pub(crate) fn volume_name(path: &str) -> Option<String> {
    drive_prefix(path).or_else(|| {
        let id = platform::device_id_of(std::path::Path::new(path))?;
        platform::device_name_of(&id)
    })
}

/// `C:` of `C:\dir` and `C:/dir`.
fn drive_prefix(path: &str) -> Option<String> {
    let mut chars = path.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':')).then(|| format!("{}:", letter.to_ascii_uppercase()))
}

fn probe() {
    // Reading the machine is a courtesy to the rest of the system.
    platform::lower_thread_priority();
    let mut previous: Option<(RawSample, Instant)> = None;
    loop {
        let watch = state().watch.clone();
        let taken = Instant::now();
        let raw = platform::read_raw(&watch);
        let sample = match &previous {
            Some((before, at)) => {
                let wall = taken.saturating_duration_since(*at).as_millis() as u64;
                sample_from(Some(before), &raw, wall)
            }
            None => sample_from(None, &raw, 0),
        };
        {
            let mut state = state();
            if state.last_request.elapsed() >= REST_AFTER {
                // Rest; a stale reading must not be served after waking.
                state.running = false;
                state.sample = unknown();
                return;
            }
            state.sample = sample;
        }
        previous = Some((raw, taken));
        std::thread::sleep(PROBE_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_sensor_drive_prefix_names_a_letter_only() {
        assert_eq!(drive_prefix(r"c:\Users\me").as_deref(), Some("C:"));
        assert_eq!(drive_prefix("D:/data").as_deref(), Some("D:"));
        assert_eq!(drive_prefix("/home/me"), None);
        assert_eq!(drive_prefix(r"\\server\share"), None);
        assert_eq!(drive_prefix("1:x"), None);
        assert!(same_volume(r"C:\a", r"c:\b"));
        assert!(!same_volume(r"C:\a", r"D:\b"));
    }

    #[test]
    fn speed_task_sensor_latest_always_answers_without_a_reading() {
        let first = latest();
        assert!(first.cores_available >= 1.0, "the parallelism stands in");
        // a path with no device behind it has no queue and no name
        assert_eq!(disk_queue_for("/nonexistent/path"), None);
        assert_eq!(volume_name("/nonexistent/path"), None);
    }
}
