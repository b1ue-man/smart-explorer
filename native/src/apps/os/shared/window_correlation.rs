//! Host-local process provenance and original P4 window witnesses. No PID-only
//! lookup, display-name matching or peer-supplied native window identifier.
use crate::apps::{LaunchRequest, RequestId};
use crate::remote_window::{NativeWindowId, WindowAdapter, WindowBinding};
use crate::screen::core::family_protocol::FamilyListEntry;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, OnceLock},
};

type ProcessAlive = Arc<dyn Fn() -> bool + Send + Sync>;
struct Started {
    request: LaunchRequest,
    key: String,
    alive: ProcessAlive,
}
fn handoff() -> &'static Mutex<BTreeMap<String, Started>> {
    static STARTED: OnceLock<Mutex<BTreeMap<String, Started>>> = OnceLock::new();
    STARTED.get_or_init(|| Mutex::new(BTreeMap::new()))
}
/// Called before closing the real Shell hProcess, or reaping the exact Child.
pub(crate) fn record(request: &LaunchRequest, dispatch: &str, key: String, alive: ProcessAlive) {
    if let Ok(mut starts) = handoff().lock() {
        starts.insert(
            dispatch.into(),
            Started {
                request: request.clone(),
                key,
                alive,
            },
        );
    }
}
struct Observer {
    adapter: Box<dyn WindowAdapter>,
    binding: WindowBinding,
    entry: Option<FamilyListEntry>,
}
impl Observer {
    fn valid(&mut self, request: &LaunchRequest, key: &str) -> io::Result<()> {
        self.adapter.validate(&self.binding)?;
        if self.adapter.inspect(&self.binding)?.process != key
            || !self
                .adapter
                .process_user_matches(&self.binding, request.user_id.as_str())?
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "original_app_process_or_user_changed",
            ));
        }
        self.adapter.validate(&self.binding)
    }
}
struct Tracked {
    request: LaunchRequest,
    key: String,
    alive: Option<ProcessAlive>,
    observer: Option<Observer>,
    ended: bool,
}
#[derive(Default)]
pub(crate) struct Correlations {
    entries: BTreeMap<RequestId, Tracked>,
}
pub(crate) struct Confirmed {
    pub entry: FamilyListEntry,
    pub focused: bool,
}
impl Correlations {
    pub(crate) fn take_started(&mut self, request: &LaunchRequest, dispatch: &str) {
        let started = handoff()
            .lock()
            .ok()
            .and_then(|mut starts| starts.remove(dispatch));
        if let Some(started) = started.filter(|started| started.request == *request) {
            self.entries.insert(
                request.request_id.clone(),
                Tracked {
                    request: request.clone(),
                    key: started.key,
                    alive: Some(started.alive),
                    observer: None,
                    ended: false,
                },
            );
        }
    }
    /// Fresh authorization/deadline/session is checked immediately before Focus.
    /// A denied foreground request does not trigger a second executable launch.
    pub(crate) fn activate_known(
        &mut self,
        request: &LaunchRequest,
        deadline: Option<i64>,
        valid: &Option<Arc<dyn Fn() -> bool + Send + Sync>>,
        session_valid: impl Fn() -> bool,
    ) -> io::Result<Option<bool>> {
        let mut candidates = BTreeMap::<u64, (NativeWindowId, RequestId, String)>::new();
        for (id, known) in &mut self.entries {
            if known.alive.as_ref().is_none_or(|alive| !alive()) {
                known.ended = true;
                known.alive = None;
                known.observer = None;
            }
            if known.ended
                || known.request.app_id != request.app_id
                || known.request.user_id != request.user_id
                || known.request.session_id != request.session_id
                || known.request.catalog_revision != request.catalog_revision
            {
                continue;
            }
            let Some(observer) = known.observer.as_mut() else {
                continue;
            };
            if observer.valid(&known.request, &known.key).is_ok() {
                candidates.insert(
                    observer.binding.native.0,
                    (observer.binding.native, id.clone(), known.key.clone()),
                );
            }
        }
        if candidates.len() != 1 {
            return Ok(None);
        }
        let Some((_, (native, anchor, key))) = candidates.into_iter().next() else {
            return Ok(None);
        };
        let mut adapter = crate::remote_window::os::adapter()?;
        let binding = adapter.bind(native)?;
        let mut observer = Observer {
            adapter,
            binding,
            entry: None,
        };
        observer.valid(request, &key)?;
        let original = self
            .entries
            .get_mut(&anchor)
            .and_then(|known| known.observer.as_mut())
            .ok_or_else(|| io::Error::other("known_window_witness_ended"))?;
        original.adapter.validate(&original.binding)?;
        let alive = self
            .entries
            .get(&anchor)
            .and_then(|known| known.alive.clone())
            .ok_or_else(|| io::Error::other("original_app_process_ended"))?;
        if !alive() {
            return Ok(None);
        }
        if !session_valid()
            || super::platform::expired(deadline)
            || valid.as_ref().is_some_and(|valid| !valid())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "app_activation_admission_ended",
            ));
        }
        let admitted =
            || !super::platform::expired(deadline) && valid.as_ref().is_none_or(|valid| valid());
        let activation = observer.adapter.activate_if(&observer.binding, &admitted);
        if activation
            .as_ref()
            .is_err_and(|error| error.to_string() == "app_activation_admission_ended")
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "app_activation_admission_ended",
            ));
        }
        let requested = activation.is_ok();
        observer.valid(request, &key)?;
        let focused = requested && observer.adapter.focused(&observer.binding).unwrap_or(false);
        if !alive() {
            return Err(io::Error::other(
                "original_app_process_ended_during_activation",
            ));
        }
        self.entries.insert(
            request.request_id.clone(),
            Tracked {
                request: request.clone(),
                key,
                alive: Some(alive),
                observer: Some(observer),
                ended: false,
            },
        );
        Ok(Some(focused))
    }
    /// Only observation and pin transfer; never starts or activates an app.
    pub(crate) fn confirm(
        &mut self,
        request: &LaunchRequest,
        register: impl FnOnce(crate::remote_window::WindowPin) -> io::Result<FamilyListEntry>,
    ) -> io::Result<Option<Confirmed>> {
        let Some(tracked) = self.entries.get_mut(&request.request_id) else {
            return Ok(None);
        };
        if tracked.request != *request {
            return Err(io::Error::other("window_request_body_changed"));
        }
        if tracked.alive.as_ref().is_none_or(|alive| !alive()) {
            tracked.ended = true;
            tracked.alive = None;
            tracked.observer = None;
        }
        if tracked.ended {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "original_app_window_lifetime_ended",
            ));
        }
        if tracked.observer.is_none() {
            let mut adapter = crate::remote_window::os::adapter()?;
            let candidates: Vec<_> = adapter
                .candidate_windows()?
                .into_iter()
                .filter(|(_, key)| *key == tracked.key)
                .collect();
            if candidates.len() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    if candidates.is_empty() {
                        "original_app_process_window_not_yet_available"
                    } else {
                        "multiple_original_process_windows_require_explicit_selection"
                    },
                ));
            }
            let binding = adapter.bind(candidates[0].0)?;
            tracked.observer = Some(Observer {
                adapter,
                binding,
                entry: None,
            });
        }
        let Some(observer) = tracked.observer.as_mut() else {
            return Ok(None);
        };
        if let Err(error) = observer.valid(request, &tracked.key) {
            tracked.ended = true;
            tracked.alive = None;
            tracked.observer = None;
            return Err(error);
        }
        if observer.entry.is_none() {
            let pin = observer.adapter.export_pin(&observer.binding)?;
            // Err means the receiver assumed no cleanup responsibility. On Ok,
            // transfer first even if a subsequent local validation fails.
            let entry = register(pin)?;
            observer.adapter.transfer_pin(&observer.binding);
            observer.entry = Some(entry);
        }
        observer.valid(request, &tracked.key)?;
        if tracked.alive.as_ref().is_none_or(|alive| !alive()) {
            return Err(io::Error::other(
                "original_app_process_ended_during_pin_transfer",
            ));
        }
        let focused = observer.adapter.focused(&observer.binding).unwrap_or(false);
        observer.valid(request, &tracked.key)?;
        Ok(observer
            .entry
            .clone()
            .map(|entry| Confirmed { entry, focused }))
    }
    pub(crate) fn has_process(&self, request: &LaunchRequest) -> bool {
        self.entries
            .get(&request.request_id)
            .is_some_and(|tracked| tracked.request == *request && !tracked.ended)
    }
}
