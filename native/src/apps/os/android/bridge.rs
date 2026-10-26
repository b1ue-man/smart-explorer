//! In-process Android bridge. Fresh nonce/response per catalog and dispatch;
//! no cached prerequisite boolean can authorize a background activity start.
use super::super::shared::platform::DispatchReceipt;
use crate::apps::core::model::{AppId, CatalogApp, HostSession, LaunchRequest, LaunchState};
use crate::screen::core::app_icon::{IconRef, IconReply};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::Duration,
};

type EventSink = fn(Value);
type ResultSink = fn(LaunchRequest, String, DispatchReceipt);
type ConsentSink = fn(LaunchRequest);
const WAIT: Duration = Duration::from_secs(10);
const MAX_PENDING: usize = 64;
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AndroidApp {
    pub app: CatalogApp,
    pub fingerprint: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AndroidCatalog {
    pub session: HostSession,
    pub apps: Vec<AndroidApp>,
    #[serde(default)]
    pub icons: Vec<IconReply>,
    #[serde(default)]
    pub limits: Vec<String>,
}
struct Reply<T> {
    value: Mutex<Option<T>>,
    ready: Condvar,
}
impl<T> Default for Reply<T> {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
            ready: Condvar::new(),
        }
    }
}
impl<T> Reply<T> {
    fn finish(&self, value: T) -> io::Result<()> {
        *self
            .value
            .lock()
            .map_err(|_| failed("bridge response mutex poisoned"))? = Some(value);
        self.ready.notify_all();
        Ok(())
    }
    fn wait(&self) -> io::Result<T> {
        self.wait_for(WAIT)
    }
    fn wait_for(&self, timeout: Duration) -> io::Result<T> {
        let value = self
            .value
            .lock()
            .map_err(|_| failed("bridge response mutex poisoned"))?;
        let (mut value, _) = self
            .ready
            .wait_timeout_while(value, timeout, |v| v.is_none())
            .map_err(|_| failed("bridge response mutex poisoned"))?;
        value.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Android launch bridge reply not yet available",
            )
        })
    }
}
struct Attempt {
    request: LaunchRequest,
    dispatch_id: String,
    reply: Arc<Reply<DispatchReceipt>>,
}
struct IconAttempt {
    expected: IconRef,
    reply: Arc<Reply<Option<IconReply>>>,
}
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
#[derive(Default)]
struct State {
    sink: Option<EventSink>,
    results: Option<ResultSink>,
    consents: Option<ConsentSink>,
    catalogs: BTreeMap<String, Arc<Reply<io::Result<AndroidCatalog>>>>,
    launches: BTreeMap<String, Attempt>,
    consent_requests: BTreeMap<String, (LaunchRequest, String)>,
    icon_requests: BTreeMap<String, IconAttempt>,
}
fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}
fn failed(reason: &str) -> io::Error {
    io::Error::other(reason)
}
fn nonce() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| failed("bridge nonce source unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn set_event_sink(sink: EventSink) -> io::Result<()> {
    state()
        .lock()
        .map_err(|_| failed("Android app bridge poisoned"))?
        .sink = Some(sink);
    Ok(())
}
/// Main stores late local replies through AppHost::note_dispatch_result. This
/// sink must enqueue its work: a synchronous AppHost callback would deadlock
/// against the admission guard while dispatch is waiting for Kotlin.
pub(crate) fn set_result_sink(sink: ResultSink) -> io::Result<()> {
    state()
        .lock()
        .map_err(|_| failed("Android app bridge poisoned"))?
        .results = Some(sink);
    Ok(())
}
/// Main resumes with the ORIGINAL authenticated peer and a fresh E4 check.
/// This is a local JNI/UI hook, never a remote capability endpoint. Like the
/// result sink it must enqueue and return, so Kotlin can receive the next event.
pub(crate) fn set_consent_sink(sink: ConsentSink) -> io::Result<()> {
    state()
        .lock()
        .map_err(|_| failed("Android app bridge poisoned"))?
        .consents = Some(sink);
    Ok(())
}
pub(super) fn catalog() -> io::Result<AndroidCatalog> {
    let challenge = nonce()?;
    let reply = Arc::new(Reply::default());
    let sink = {
        let mut state = state()
            .lock()
            .map_err(|_| failed("Android app bridge poisoned"))?;
        let sink = state
            .sink
            .ok_or_else(|| failed("Android application bridge is not registered"))?;
        if state.catalogs.len() >= MAX_PENDING {
            return Err(failed("Android catalog bridge is busy"));
        }
        state.catalogs.insert(challenge.clone(), reply.clone());
        sink
    };
    sink(json!({"type":"appsHostCatalog","challenge":challenge}));
    let result = reply.wait();
    state()
        .lock()
        .map_err(|_| failed("Android app bridge poisoned"))?
        .catalogs
        .remove(&challenge);
    result?
}
pub(crate) fn publish_catalog(challenge: &str, catalog: AndroidCatalog) -> io::Result<()> {
    if serde_json::to_vec(&catalog)
        .map_err(io::Error::other)?
        .len()
        > MAX_CATALOG_BYTES
        || catalog.apps.len() > 8192
    {
        return Err(failed("Android catalog bridge byte/entry budget exceeded"));
    }
    let mut icons = BTreeMap::new();
    for reply in &catalog.icons {
        reply.bytes_for(&reply.icon)?;
        if !catalog.apps.iter().any(|a| {
            a.app.app_id.as_str() == reply.icon.app_id && a.app.icon.as_ref() == Some(&reply.icon)
        }) {
            return Err(failed(
                "Android icon is not bound to its current launcher entry",
            ));
        }
        if icons
            .insert(reply.icon.app_id.clone(), reply.clone())
            .is_some()
        {
            return Err(failed("duplicate Android catalog icon"));
        }
    }
    let reply = {
        let mut state = state()
            .lock()
            .map_err(|_| failed("Android app bridge poisoned"))?;
        let reply = state
            .catalogs
            .remove(challenge)
            .ok_or_else(|| failed("unknown/expired Android catalog challenge"))?;
        reply
    };
    reply.finish(Ok(catalog))
}
pub(crate) fn catalog_failed(challenge: &str, reason: &str) -> io::Result<()> {
    if reason.len() > 4096 {
        return Err(failed("Android catalog failure detail too long"));
    }
    let reply = state()
        .lock()
        .map_err(|_| failed("Android app bridge poisoned"))?
        .catalogs
        .remove(challenge)
        .ok_or_else(|| failed("unknown/expired Android catalog challenge"))?;
    reply.finish(Err(io::Error::new(io::ErrorKind::Unsupported, reason)))
}
pub(super) fn dispatch(
    request: &LaunchRequest,
    expected: &HostSession,
    fingerprint: &str,
    deadline: Option<i64>,
    dispatch_id: &str,
) -> io::Result<DispatchReceipt> {
    let challenge = nonce()?;
    let reply = Arc::new(Reply::default());
    let sink = {
        let mut state = state()
            .lock()
            .map_err(|_| failed("Android app bridge poisoned"))?;
        let sink = state
            .sink
            .ok_or_else(|| failed("Android application bridge is not registered"))?;
        if state.launches.len() + state.consent_requests.len() >= MAX_PENDING {
            return Ok(DispatchReceipt {
                state: LaunchState::Unsupported,
                reason: "android_local_launch_queue_budget_exceeded".into(),
                pid: None,
            });
        }
        state.launches.insert(
            challenge.clone(),
            Attempt {
                request: request.clone(),
                dispatch_id: dispatch_id.into(),
                reply: reply.clone(),
            },
        );
        sink
    };
    sink(
        json!({"type":"appsHostLaunch","challenge":challenge,"request":request,"expected_session":expected,
        "fingerprint":fingerprint,"expires_at_unix_ms":deadline,"dispatch_id":dispatch_id}),
    );
    // On timeout keep the attempt: a late local reply can settle the durable
    // OutcomeUnknown receipt, and must never lead to an automatic retry.
    match reply.wait() {
        Err(error) if error.kind() == io::ErrorKind::TimedOut => {
            let end = admissions()
                .lock()
                .map_err(|_| failed("Android admission registry poisoned"))?
                .get(dispatch_id)
                .and_then(|a| a.claimed_until);
            if let Some(end) = end {
                let remaining = end.saturating_duration_since(std::time::Instant::now());
                if !remaining.is_zero() {
                    return reply.wait_for(remaining);
                }
            }
            Err(error)
        }
        result => result,
    }
}
pub(crate) fn finish_launch(
    challenge: &str,
    status: LaunchState,
    reason: String,
) -> io::Result<()> {
    if !matches!(
        status,
        LaunchState::OutcomeUnknown
            | LaunchState::StartedWindowPending
            | LaunchState::ConsentRequired
            | LaunchState::Denied
            | LaunchState::Unsupported
    ) || reason.len() > 4096
    {
        return Err(failed("invalid local Android launch outcome"));
    }
    let (attempt, result_sink) = {
        let mut state = state()
            .lock()
            .map_err(|_| failed("Android app bridge poisoned"))?;
        let attempt = state
            .launches
            .remove(challenge)
            .ok_or_else(|| failed("unknown/expired Android launch challenge"))?;
        if status == LaunchState::ConsentRequired {
            state.consent_requests.insert(
                attempt.request.request_id.to_string(),
                (attempt.request.clone(), attempt.dispatch_id.clone()),
            );
        }
        (attempt, state.results)
    };
    let result = DispatchReceipt {
        state: status.clone(),
        reason: reason.clone(),
        pid: None,
    };
    attempt.reply.finish(result)?;
    if let Some(sink) = result_sink {
        sink(
            attempt.request,
            attempt.dispatch_id,
            DispatchReceipt {
                state: status,
                reason,
                pid: None,
            },
        );
    }
    Ok(())
}
pub(crate) fn confirm_local(request_id: &str) -> io::Result<()> {
    let (request, sink) = {
        let mut state = state()
            .lock()
            .map_err(|_| failed("Android app bridge poisoned"))?;
        let sink = state
            .consents
            .ok_or_else(|| failed("local app consent admission is not installed"))?;
        let (request, _dispatch) = state
            .consent_requests
            .remove(request_id)
            .ok_or_else(|| failed("no pending local app consent"))?;
        (request, sink)
    };
    sink(request);
    Ok(())
}
pub(crate) fn deny_local(request_id: &str) -> io::Result<()> {
    let (request, dispatch, sink) = {
        let mut state = state()
            .lock()
            .map_err(|_| failed("Android app bridge poisoned"))?;
        let (request, dispatch) = state
            .consent_requests
            .remove(request_id)
            .ok_or_else(|| failed("no pending local app consent"))?;
        (
            request,
            dispatch,
            state
                .results
                .ok_or_else(|| failed("local app result sink is not installed"))?,
        )
    };
    sink(
        request,
        dispatch,
        DispatchReceipt {
            state: LaunchState::Denied,
            reason: "host_user_refused_application_start".into(),
            pid: None,
        },
    );
    Ok(())
}
pub(crate) fn end_local_consent(request: &LaunchRequest, reason: &str) {
    let sink = state().lock().ok().and_then(|mut state| {
        state.consent_requests.remove(request.request_id.as_str());
        state.sink
    });
    if let Some(sink) = sink {
        sink(json!({"type":"appsHostConsentEnded","request":request,"reason":reason}));
    }
}
pub(super) fn icon(id: &AppId, reference: &IconRef) -> io::Result<Option<IconReply>> {
    if id.as_str() != reference.app_id {
        return Err(failed("Android icon app identity mismatch"));
    }
    reference.validate()?;
    let challenge = nonce()?;
    let reply = Arc::new(Reply::default());
    let sink = {
        let mut state = state()
            .lock()
            .map_err(|_| failed("Android app bridge poisoned"))?;
        let sink = state
            .sink
            .ok_or_else(|| failed("Android application bridge is not registered"))?;
        if state.icon_requests.len() >= MAX_PENDING {
            return Err(failed("Android icon bridge busy"));
        }
        state.icon_requests.insert(
            challenge.clone(),
            IconAttempt {
                expected: reference.clone(),
                reply: reply.clone(),
            },
        );
        sink
    };
    sink(json!({"type":"appsHostIcon","challenge":challenge,"app_id":id,"icon":reference}));
    let result = reply.wait();
    state()
        .lock()
        .map_err(|_| failed("Android app bridge poisoned"))?
        .icon_requests
        .remove(&challenge);
    result
}
pub(crate) fn finish_icon(challenge: &str, reply: Option<IconReply>) -> io::Result<()> {
    let attempt = state()
        .lock()
        .map_err(|_| failed("Android app bridge poisoned"))?
        .icon_requests
        .remove(challenge)
        .ok_or_else(|| failed("unknown/expired Android icon challenge"))?;
    if let Some(reply) = &reply {
        reply.bytes_for(&attempt.expected)?;
    }
    attempt.reply.finish(reply)
}
