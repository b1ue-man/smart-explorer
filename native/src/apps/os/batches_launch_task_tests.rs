//! B8: real AppHost and durable Journal; only OS catalog/dispatch are simulated.
use super::super::shared::platform::{DispatchReceipt, InstalledApp};
use super::*;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Clone)]
struct FakePlatform {
    session: Arc<Mutex<HostSession>>,
    fingerprint: Arc<Mutex<String>>,
    results: Arc<Mutex<VecDeque<Option<LaunchState>>>>,
    dispatches: Arc<AtomicUsize>,
    guard_held: Arc<AtomicBool>,
    ledger: PathBuf,
}
impl FakePlatform {
    fn new(ledger: &Path) -> Self {
        Self {
            session: Arc::new(Mutex::new(HostSession {
                user_id: "user-1".to_string().try_into().unwrap(),
                session_id: "logon-1".to_string().try_into().unwrap(),
                interactive: true,
                unlocked: true,
            })),
            fingerprint: Arc::new(Mutex::new("installed-editor-v1".into())),
            results: Arc::new(Mutex::new(VecDeque::new())),
            dispatches: Arc::new(AtomicUsize::new(0)),
            guard_held: Arc::new(AtomicBool::new(false)),
            ledger: ledger.to_owned(),
        }
    }
}
impl HostPlatform for FakePlatform {
    type Target = String;
    fn snapshot(&self) -> io::Result<PlatformCatalog<Self::Target>> {
        Ok(PlatformCatalog {
            session: self.session.lock().unwrap().clone(),
            apps: vec![InstalledApp {
                app: CatalogApp {
                    app_id: "editor".to_string().try_into().unwrap(),
                    name: "Editor".into(),
                    kind: AppKind::LinuxDesktopEntry,
                    icon: None,
                    can_launch: true,
                    unavailable_reason: None,
                },
                fingerprint: self.fingerprint.lock().unwrap().clone(),
                target: "host-resolved-editor".into(),
            }],
            limits: Vec::new(),
        })
    }
    fn dispatch(
        &self,
        target: &String,
        request: &LaunchRequest,
        expected: &HostSession,
        _: Option<i64>,
        dispatch_id: &str,
    ) -> io::Result<DispatchReceipt> {
        assert_eq!(target, "host-resolved-editor");
        assert_eq!(expected, &*self.session.lock().unwrap());
        assert!(
            self.guard_held.load(Ordering::SeqCst),
            "admission guard must survive OS dispatch"
        );
        let receipt = Journal::new(&self.ledger)?
            .read(&request.request_id)?
            .unwrap();
        assert_eq!(receipt.request, *request);
        assert_eq!(
            receipt.state,
            LaunchState::OutcomeUnknown,
            "claim must be durable before OS effect"
        );
        assert_eq!(receipt.dispatch_id, dispatch_id);
        assert!(!dispatch_id.is_empty());
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        match self
            .results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Some(LaunchState::StartedWindowPending))
        {
            Some(state) => Ok(DispatchReceipt {
                state,
                reason: "fake OS result".into(),
                pid: Some(41),
            }),
            None => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "OS reply lost after dispatch",
            )),
        }
    }
}

struct Held(Arc<AtomicBool>);
impl Drop for Held {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
fn admit(platform: &FakePlatform) -> LaunchAdmission<Held> {
    platform.guard_held.store(true, Ordering::SeqCst);
    LaunchAdmission {
        guard: Held(platform.guard_held.clone()),
        expires_at_unix_ms: None,
        valid: None,
    }
}
fn host(platform: &FakePlatform) -> AppHost<FakePlatform> {
    AppHost::new(
        "device-1".to_string().try_into().unwrap(),
        platform.clone(),
        &platform.ledger,
    )
    .unwrap()
}
fn request(host: &AppHost<FakePlatform>, id: &str) -> LaunchRequest {
    let catalog = host.catalog().unwrap();
    LaunchRequest {
        request_id: id.to_string().try_into().unwrap(),
        target_device_id: catalog.device_id,
        app_id: catalog.apps[0].app_id.clone(),
        user_id: catalog.session.user_id,
        session_id: catalog.session.session_id,
        catalog_revision: catalog.revision,
    }
}

#[test]
fn batches_app_durable_claim_precedes_dispatch_and_survives_host_reconstruction() {
    let root = tempfile::tempdir().unwrap();
    let platform = FakePlatform::new(root.path());
    let first = host(&platform);
    let request = request(&first, "repeat-after-restart");
    let receipt = first.launch(&request, |_| Ok(admit(&platform))).unwrap();
    assert_eq!(receipt.state, LaunchState::StartedWindowPending);
    assert!(!platform.guard_held.load(Ordering::SeqCst));
    drop(first);
    let restarted = host(&platform);
    assert_eq!(
        restarted
            .launch(&request, |_| Ok(admit(&platform)))
            .unwrap(),
        receipt
    );
    assert_eq!(restarted.receipt(&request).unwrap(), Some(receipt));
    assert_eq!(platform.dispatches.load(Ordering::SeqCst), 1);
}

#[test]
fn batches_app_ambiguous_dispatch_remains_unknown_until_matching_completion() {
    let root = tempfile::tempdir().unwrap();
    let platform = FakePlatform::new(root.path());
    platform.results.lock().unwrap().push_back(None);
    let first = host(&platform);
    let request = request(&first, "lost-os-reply");
    let unknown = first.launch(&request, |_| Ok(admit(&platform))).unwrap();
    assert_eq!(unknown.state, LaunchState::OutcomeUnknown);
    drop(first);
    let restarted = host(&platform);
    assert_eq!(
        restarted
            .launch(&request, |_| Ok(admit(&platform)))
            .unwrap(),
        unknown
    );
    let result = || DispatchReceipt {
        state: LaunchState::StartedWindowPending,
        reason: "late exact OS result".into(),
        pid: Some(41),
    };
    assert_eq!(
        restarted
            .note_dispatch_result(&request, "stale-dispatch", result())
            .unwrap(),
        unknown
    );
    let forged_window = DispatchReceipt {
        state: LaunchState::Activated,
        reason: "unproved window".into(),
        pid: None,
    };
    assert!(restarted
        .note_dispatch_result(&request, &unknown.dispatch_id, forged_window)
        .is_err());
    assert_eq!(restarted.receipt(&request).unwrap(), Some(unknown.clone()));
    let settled = restarted
        .note_dispatch_result(&request, &unknown.dispatch_id, result())
        .unwrap();
    assert_eq!(settled.state, LaunchState::StartedWindowPending);
    assert_eq!(platform.dispatches.load(Ordering::SeqCst), 1);
}

#[test]
fn batches_app_peer_owner_and_complete_body_survive_reconstruction_without_rebinding() {
    let root = tempfile::tempdir().unwrap();
    let platform = FakePlatform::new(root.path());
    let first = host(&platform);
    let request = request(&first, "owned-request");
    let owner = "ab".repeat(32);
    let foreign = "cd".repeat(32);
    assert!(!first.bind_peer_request(&request, &owner, false).unwrap());
    assert!(first.bind_peer_request(&request, &owner, true).unwrap());
    drop(first);
    let restarted = host(&platform);
    assert!(restarted
        .bind_peer_request(&request, &owner, false)
        .unwrap());
    assert!(!restarted
        .bind_peer_request(&request, &foreign, true)
        .unwrap());
    for case in 0..5 {
        let mut changed = request.clone();
        match case {
            0 => changed.app_id = "terminal".to_string().try_into().unwrap(),
            1 => changed.user_id = "user-2".to_string().try_into().unwrap(),
            2 => changed.session_id = "logon-2".to_string().try_into().unwrap(),
            3 => changed.catalog_revision = "ff".repeat(32),
            4 => changed.target_device_id = "device-2".to_string().try_into().unwrap(),
            _ => unreachable!(),
        }
        assert!(
            !restarted.bind_peer_request(&changed, &owner, true).unwrap(),
            "case {case}"
        );
        let denied = restarted
            .launch::<()>(&changed, |_| {
                panic!("changed request reached authorization")
            })
            .unwrap();
        assert_eq!(denied.state, LaunchState::Denied);
        assert_eq!(denied.reason, "request_id_reused_with_different_request");
    }
    assert_eq!(platform.dispatches.load(Ordering::SeqCst), 0);
}

#[test]
fn batches_app_catalog_logon_revocation_and_expiry_prevent_dispatch() {
    for case in 0..5 {
        let root = tempfile::tempdir().unwrap();
        let platform = FakePlatform::new(root.path());
        let host = host(&platform);
        let request = request(&host, "admission-changed");
        match case {
            0 => *platform.fingerprint.lock().unwrap() = "installed-editor-v2".into(),
            1 => {
                platform.session.lock().unwrap().session_id =
                    "logon-2".to_string().try_into().unwrap()
            }
            2 => {
                platform.session.lock().unwrap().user_id = "user-2".to_string().try_into().unwrap()
            }
            _ => {}
        }
        let denied = host
            .launch(&request, |_| {
                assert!(
                    case >= 3,
                    "changed catalog/login must fail before grant lookup"
                );
                let mut admission = admit(&platform);
                if case == 3 {
                    admission.expires_at_unix_ms = Some(0);
                }
                if case == 4 {
                    admission.valid = Some(Arc::new(|| false));
                }
                Ok(admission)
            })
            .unwrap();
        assert_eq!(denied.state, LaunchState::Denied, "case {case}");
        assert_eq!(platform.dispatches.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn batches_app_consent_continuation_uses_new_phase_and_never_repeats_remote_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let platform = FakePlatform::new(root.path());
    platform
        .results
        .lock()
        .unwrap()
        .extend([Some(LaunchState::ConsentRequired), None]);
    let host = host(&platform);
    let request = request(&host, "local-consent");
    let pending = host.launch(&request, |_| Ok(admit(&platform))).unwrap();
    assert_eq!(pending.state, LaunchState::ConsentRequired);
    assert_eq!(
        host.launch(&request, |_| Ok(admit(&platform))).unwrap(),
        pending
    );
    assert_eq!(platform.dispatches.load(Ordering::SeqCst), 1);
    let resumed = host
        .resume_after_local_consent(&request, |_| Ok(admit(&platform)))
        .unwrap();
    assert_eq!(resumed.state, LaunchState::OutcomeUnknown);
    assert_ne!(resumed.dispatch_id, pending.dispatch_id);
    let stale = DispatchReceipt {
        state: LaunchState::Denied,
        reason: "old consent rejected".into(),
        pid: None,
    };
    assert_eq!(
        host.note_dispatch_result(&request, &pending.dispatch_id, stale)
            .unwrap(),
        resumed
    );
    assert_eq!(
        host.resume_after_local_consent(&request, |_| Ok(admit(&platform)))
            .unwrap(),
        resumed
    );
    assert_eq!(platform.dispatches.load(Ordering::SeqCst), 2);
}

#[test]
fn batches_app_repeated_request_rechecks_permission_before_disclosing_receipt() {
    let root = tempfile::tempdir().unwrap();
    let platform = FakePlatform::new(root.path());
    let host = host(&platform);
    let request = request(&host, "permission-revoked");
    let original = host.launch(&request, |_| Ok(admit(&platform))).unwrap();
    let denied = host
        .launch::<()>(&request, |_| {
            Err(LaunchDenial::Denied("grant revoked".into()))
        })
        .unwrap();
    assert_eq!(denied.state, LaunchState::Denied);
    assert_eq!(denied.reason, "grant revoked");
    assert_eq!(host.receipt(&request).unwrap(), Some(original));
    assert_eq!(platform.dispatches.load(Ordering::SeqCst), 1);
}
