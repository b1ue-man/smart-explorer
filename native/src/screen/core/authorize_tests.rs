use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::share::access_approval::{
    ApprovalDecision, ApprovalService, ApprovalView, ConsentNotifier, NoConsent,
};
use crate::share::exec_access_view::ApprovalSurface;

struct Window;

impl ConsentNotifier for Window {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        Some(ApprovalSurface::Window)
    }
}

struct Rights {
    decision: Mutex<CapabilityDecision>,
}

impl ScreenAuthority for Rights {
    fn capability(
        &self,
        _principal: &ProvenPrincipal,
        _capability: Capability,
        _now: i64,
    ) -> Option<CapabilityState> {
        Some(CapabilityState {
            decision: *self.decision.lock().unwrap(),
            policy_revision: 3,
            changed_at: 0,
        })
    }

    fn admit(
        &self,
        _principal: &ProvenPrincipal,
        _needed: &[Capability],
        _now: i64,
        admit: &mut dyn FnMut(),
    ) -> bool {
        admit();
        true
    }

    fn stored_names(&self, _principal: &ProvenPrincipal) -> (Option<String>, Option<String>) {
        (Some("Laptop".into()), None)
    }
}

static LINES: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());

fn capture(entry: AccessEntry<'_>) {
    let line = crate::share::access_audit::access_line(&entry);
    LINES.lock().unwrap().push(line);
}

fn principal() -> ProvenPrincipal {
    ProvenPrincipal {
        relation_kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: "authorize-test-laptop".into(),
        device_name: "Laptop".into(),
        public_key: "key-laptop".into(),
        fingerprint: "fp-laptop".into(),
        node_id: "key-laptop".into(),
    }
}

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// Asks once; `answer` gets the service as soon as the request is open.
fn asked(
    service: &ApprovalService,
    rights: &Rights,
    gone: bool,
    answer: impl FnOnce(&ApprovalService) + Send + 'static,
) -> Approval {
    let broker: Arc<dyn ConsentBroker> = Arc::new(service.broker());
    let waiter = service.clone();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let decider = std::thread::spawn(move || {
        while !finished.load(Ordering::Acquire) {
            if !waiter.pending().is_empty() {
                answer(&waiter);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let principal = principal();
    let (_stop, signal) = watch::channel(None);
    let context = AskContext {
        authority: rights,
        broker: &broker,
        audit: capture,
        principal: &principal,
        mode: SessionMode::View,
        capability: Capability::ScreenView,
        policy_revision: 3,
    };
    let approval = run(async {
        let viewer_gone = async move {
            if !gone {
                std::future::pending::<()>().await;
            }
        };
        ask(context, async {}, viewer_gone, signal).await
    });
    done.store(true, Ordering::Release);
    let _ = decider.join();
    approval
}

#[test]
fn screen_task_authorize_decide_and_ask() {
    use CapabilityDecision::{Allow, Ask, Off};
    use SessionDecision::{Allowed, InfoOnly, Refused};
    let ask_view = SessionDecision::Ask(Capability::ScreenView);
    let ask_control = SessionDecision::Ask(Capability::ScreenControl);
    let cases = [
        (
            SessionMode::View,
            Off,
            Allow,
            Refused(ScreenErrorCode::PermissionDenied),
        ),
        (
            SessionMode::Probe,
            Off,
            Off,
            Refused(ScreenErrorCode::PermissionDenied),
        ),
        (SessionMode::Probe, Ask, Off, InfoOnly),
        (SessionMode::Preview, Ask, Off, InfoOnly),
        (SessionMode::Preview, Allow, Off, Allowed),
        (SessionMode::View, Ask, Off, ask_view),
        (SessionMode::View, Allow, Off, Allowed),
        (
            SessionMode::Control,
            Allow,
            Off,
            Refused(ScreenErrorCode::RightNotGranted),
        ),
        (SessionMode::Control, Allow, Ask, ask_control),
        (SessionMode::Control, Ask, Allow, ask_view),
        (SessionMode::Control, Ask, Ask, ask_control),
        (SessionMode::Control, Allow, Allow, Allowed),
    ];
    for (mode, view, control, expected) in cases {
        assert_eq!(
            decide(mode, view, control),
            expected,
            "{mode:?} {view:?} {control:?}"
        );
    }

    let rights = Rights {
        decision: Mutex::new(Ask),
    };
    let service = ApprovalService::new(Arc::new(Window));
    // „Diesmal erlauben“ and „Ablehnen“ answer exactly this request.
    let once = asked(&service, &rights, false, |service| {
        let id = service.pending()[0].id.clone();
        service.decide(&id, ApprovalDecision::Once).unwrap();
    });
    assert_eq!(once, Approval::Granted(ApprovalKind::Once));
    let denied = asked(&service, &rights, false, |service| {
        let id = service.pending()[0].id.clone();
        service.decide(&id, ApprovalDecision::Deny).unwrap();
    });
    assert_eq!(denied, Approval::Refused(ScreenErrorCode::ApprovalDenied));
    // The viewer withdraws: the request is gone from the host.
    let cancelled = asked(&service, &rights, true, |_| {});
    assert_eq!(
        cancelled,
        Approval::Refused(ScreenErrorCode::ApprovalCancelled)
    );
    assert!(service.pending().is_empty());
    // A right that allows meanwhile ends the wait as allowed.
    let allowing = Arc::new(Rights {
        decision: Mutex::new(Ask),
    });
    let switch = allowing.clone();
    let allowed = asked(&service, &allowing, false, move |_| {
        *switch.decision.lock().unwrap() = Allow;
    });
    assert_eq!(allowed, Approval::Granted(ApprovalKind::Always));
    // Nobody can be asked.
    let nobody = ApprovalService::new(Arc::new(NoConsent));
    let unavailable = asked(&nobody, &rights, false, |_| {});
    assert_eq!(
        unavailable,
        Approval::Refused(ScreenErrorCode::ApprovalUnavailable)
    );

    let lines = LINES.lock().unwrap();
    let ours: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|line| line["device_id"] == "authorize-test-laptop")
        .collect();
    let events: Vec<&str> = ours
        .iter()
        .filter_map(|line| line["event"].as_str())
        .collect();
    assert!(events.contains(&"approval_requested"), "{events:?}");
    assert!(events.contains(&"approval_granted"));
    assert!(events.contains(&"approval_denied"));
    assert!(events.contains(&"approval_cancelled"));
    assert!(events.contains(&"approval_unavailable"));
    assert!(ours
        .iter()
        .all(|line| line["kind"] == "screen" && line["mode"] == "view"));
    assert!(ours.iter().any(|line| line["approval"] == "once"));
}

#[test]
fn screen_task_authorize_stop_ends_the_wait() {
    let rights = Rights {
        decision: Mutex::new(CapabilityDecision::Ask),
    };
    let service = ApprovalService::new(Arc::new(Window));
    let broker: Arc<dyn ConsentBroker> = Arc::new(service.broker());
    let principal = principal();
    let (stop, signal) = watch::channel(None);
    stop.send_replace(Some(EndReason::CapabilityNarrowed));
    let context = AskContext {
        authority: &rights,
        broker: &broker,
        audit: capture,
        principal: &principal,
        mode: SessionMode::Control,
        capability: Capability::ScreenControl,
        policy_revision: 1,
    };
    let ended = run(ask(context, async {}, std::future::pending(), signal));
    assert_eq!(ended, Approval::Ended(EndReason::CapabilityNarrowed));
    assert!(service.pending().is_empty(), "no request was opened");
}
