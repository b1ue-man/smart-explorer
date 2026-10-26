//! One native Android launch admission, retained through its actual invocation window.
use super::failed;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

struct Admission {
    request_id: String,
    deadline: Option<i64>,
    valid: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    claimed_until: Option<std::time::Instant>,
}
#[derive(Serialize)]
pub(crate) struct AdmissionPermit {
    pub expires_at_unix_ms: i64,
    pub valid_for_ms: u64,
}
fn admissions() -> &'static Mutex<BTreeMap<String, Admission>> {
    static A: OnceLock<Mutex<BTreeMap<String, Admission>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(BTreeMap::new()))
}
pub(crate) struct AdmissionHold(String);
impl Drop for AdmissionHold {
    fn drop(&mut self) {
        let until = admissions()
            .lock()
            .ok()
            .and_then(|mut entries| entries.remove(&self.0))
            .and_then(|entry| entry.claimed_until);
        // Keep the caller's real E4 guard alive for the whole granted invocation
        // window, even if a duplicate/late result races the Java start function.
        if let Some(until) = until {
            let remaining = until.saturating_duration_since(std::time::Instant::now());
            if !remaining.is_zero() {
                std::thread::sleep(remaining);
            }
        }
    }
}
pub(crate) fn hold_admission(
    id: &str,
    request: &str,
    deadline: Option<i64>,
    valid: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
) -> io::Result<AdmissionHold> {
    let mut entries = admissions()
        .lock()
        .map_err(|_| failed("Android admission registry poisoned"))?;
    if entries.contains_key(id) {
        return Err(failed("Android dispatch admission already exists"));
    }
    entries.insert(
        id.into(),
        Admission {
            request_id: request.into(),
            deadline,
            valid,
            claimed_until: None,
        },
    );
    Ok(AdmissionHold(id.into()))
}
/// One local JNI claim. The native caller retains its guard through this short
/// invocation window; removing an admission makes delayed events inert.
pub(crate) fn claim_admission(id: &str, request: &str) -> io::Result<AdmissionPermit> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut entries = admissions()
        .lock()
        .map_err(|_| failed("Android admission registry poisoned"))?;
    let entry = entries
        .get_mut(id)
        .ok_or_else(|| failed("Android dispatch expired or cancelled"))?;
    if entry.request_id != request
        || entry.claimed_until.is_some()
        || entry.valid.as_ref().is_some_and(|check| !check())
        || entry.deadline.is_some_and(|end| now >= end)
    {
        return Err(failed("Android dispatch admission refused"));
    }
    let end = entry.deadline.map_or(now.saturating_add(2000), |end| {
        end.min(now.saturating_add(2000))
    });
    let valid_for_ms =
        u64::try_from(end.saturating_sub(now)).map_err(|_| failed("invalid dispatch deadline"))?;
    entry.claimed_until =
        std::time::Instant::now().checked_add(Duration::from_millis(valid_for_ms));
    if entry.claimed_until.is_none() {
        return Err(failed("Android dispatch deadline overflow"));
    }
    Ok(AdmissionPermit {
        expires_at_unix_ms: end,
        valid_for_ms,
    })
}

/// The existing timeout path only needs the claimed invocation deadline.
pub(super) fn claimed_until(id: &str) -> io::Result<Option<Instant>> {
    Ok(admissions()
        .lock()
        .map_err(|_| failed("Android admission registry poisoned"))?
        .get(id)
        .and_then(|entry| entry.claimed_until))
}
