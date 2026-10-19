//! PRM-M3, second part: neither a device nor the host is limited in open
//! requests (Exec and screen) or running jobs (R15); a grant narrowed while its request is open; the `cli_only`
//! surface of a host without a graphical session. The host fixture is in
//! `access_approval_tests.rs`.
use std::io;
use std::path::Path;

use super::*;
use crate::share::access_grant::CapabilityDecision::Off;
use crate::share::consent_launch::{gui_program, DesktopConsent, SessionFacts};
use crate::share::exec_client_messages::{approval_status_line, client_failure_message};

#[test]
fn perm_task_open_requests_have_no_device_or_host_limit() {
    let host = Host::new(&[(Capability::ExecCommands, Ask)]);
    let device = |id: &str| ProvenPrincipal {
        device_id: id.into(),
        ..ProvenPrincipal::from(&host.authorized.principal)
    };
    let service = &host.service;
    // One device: five open requests.
    let mut tickets = Vec::new();
    for _ in 0..5 {
        let ticket = service.request(
            &device("x"),
            Capability::ExecCommands,
            ApprovalSummary::default(),
        );
        assert_eq!(ticket.surface(), Some(ApprovalSurface::Window));
        tickets.push(ticket);
    }
    // The host: twelve open requests in total, more than the former eight.
    for index in 0..7 {
        let ticket = service.request(
            &device(&format!("other-{index}")),
            Capability::ExecTerminal,
            ApprovalSummary::default(),
        );
        assert_eq!(ticket.surface(), Some(ApprovalSurface::Window));
        tickets.push(ticket);
    }
    assert_eq!(service.pending().len(), 12);
    drop(tickets);
    assert!(service.pending().is_empty());
}

#[test]
fn perm_task_ask_start_beside_three_running_jobs_of_the_device() {
    let host = Host::new(&[(Capability::ExecCommands, Ask)]);
    let _running: Vec<_> = ["e0", "e1", "e2"]
        .iter()
        .map(|byte| host.running(&shell(byte, "sleep 60")))
        .collect();
    let start = shell("e3", "ls");
    let (decided, _, _) = drive(
        &host,
        &start,
        &client_features(),
        Act::Answer(ApprovalDecision::Once),
    );
    approved(decided);
    let admission = host.registry.prepare(
        host.authorized.principal.clone(),
        host.authorized.authorization.clone(),
        &start,
        NOW,
    );
    assert!(matches!(admission, Ok(ExecAdmission::Prepared(_))));
    assert_eq!(host.registry.active_views().len(), 4);
}

#[test]
fn perm_task_screen_request_beside_running_jobs() {
    let host = Host::new(&[(Capability::ExecCommands, Allow)]);
    host.running(&shell("f0", "sleep 60"));
    host.running(&shell("f1", "sleep 60"));
    // S4 asks through the broker: `request(principal, capability, summary)`.
    let broker = host.service.broker();
    let principal = ProvenPrincipal::from(&host.authorized.principal);
    let summary = ApprovalSummary {
        device_name: Some("Testgerät".into()),
        ..ApprovalSummary::default()
    };
    let mut screen = broker.request(&principal, Capability::ScreenView, summary.clone());
    assert_eq!(screen.surface(), Some(ApprovalSurface::Window));
    let more = broker.request(&principal, Capability::ScreenControl, summary);
    assert_eq!(more.surface(), Some(ApprovalSurface::Window));
    let open = host.service.pending();
    assert_eq!(open.len(), 2);
    assert!(open.iter().all(|view| view.command.is_none()));
    let id = screen.id().clone();
    host.service.decide(&id, ApprovalDecision::Once).unwrap();
    assert_eq!(run(screen.outcome()), ApprovalOutcome::Once);
    let admission = host.registry.prepare(
        host.authorized.principal.clone(),
        host.authorized.authorization.clone(),
        &shell("f2", "ls"),
        NOW,
    );
    assert!(matches!(admission, Ok(ExecAdmission::Prepared(_))));
}

#[test]
fn perm_task_old_host_limit_refusal_stays_readable() {
    // Hosts before R15 refuse with `admission_denied` and the bare variant.
    for (old, scope) in [("PrincipalLimit", "this device"), ("GlobalLimit", "Host")] {
        let text = client_failure_message("admission_denied", old, "Host");
        assert!(text.starts_with("Host refused the start:"), "{text}");
        assert!(text.contains(scope), "{text}");
        assert!(text.contains("Next step"), "{text}");
    }
    // Every other refusal keeps the host's own text.
    assert_eq!(
        client_failure_message("admission_denied", "DuplicateMismatch", "Host"),
        "DuplicateMismatch"
    );
}

#[test]
fn perm_task_narrowing_withdraws_pending() {
    let host = Host::new(&[
        (Capability::ExecCommands, Ask),
        (Capability::ExecTerminal, Ask),
    ]);
    let start = shell("a7", "ls");
    let narrowed = Act::Grant(vec![(Capability::ExecCommands, Off)]);
    let (decided, _, view) = drive(&host, &start, &client_features(), narrowed);
    let refusal = refused(decided);
    // Commands are off, the terminal is not: the start gets that answer.
    assert_eq!(refusal.error.code, RIGHT_NOT_GRANTED);
    assert!(host.service.pending().is_empty());
    assert!(host.consent.withdrawn.lock().unwrap().contains(&view.id));
    assert_eq!(host.events(), ["approval_requested", "approval_cancelled"]);
    assert_eq!(
        host.log.lock().unwrap()[1]["reason"],
        "the grant was narrowed"
    );
    // Another account narrows a request on „Fragen“ as well; a widening ends
    // it as allowed; an unchanged grant keeps it.
    let asked = host.auth.lock().unwrap().direct_grants[0].exec.clone();
    let principal = ProvenPrincipal::from(&host.authorized.principal);
    let changed = |grant: ExecGrant| {
        host.auth.lock().unwrap().direct_grants[0].exec = grant;
        grant_change(
            &host.auth,
            &principal,
            Capability::ExecTerminal,
            &asked,
            NOW + 5,
        )
    };
    assert_eq!(changed(asked.clone()), None);
    let mut remote = asked.clone();
    let account = AccessGrantChange {
        account: Some(crate::share::access_grant::AccountProfile::Remote),
        ..AccessGrantChange::default()
    };
    remote.apply_access_change(&account, NOW + 2).unwrap();
    assert_eq!(changed(remote), Some(GrantChange::Narrowed));
    let mut allowed = asked.clone();
    allowed
        .apply_access_change(&change(&[(Capability::ExecTerminal, Allow)]), NOW + 3)
        .unwrap();
    assert_eq!(changed(allowed), Some(GrantChange::Allows));
    let mut ended = asked.clone();
    let end = AccessGrantChange {
        expiry: Some(crate::share::access_grant::Expiry::At(NOW + 4)),
        ..AccessGrantChange::default()
    };
    ended.apply_access_change(&end, NOW + 4).unwrap();
    assert_eq!(changed(ended), Some(GrantChange::Ended));
}

#[test]
fn perm_task_pending_surface_cli_only_without_display() {
    // No `DISPLAY`, no `WAYLAND_DISPLAY` (root worker, server): nobody is
    // asked actively, neither a window nor a notification starts.
    let os = std::ffi::OsStr::new;
    let unset = SessionFacts::from_values(None, None, false);
    let empty = SessionFacts::from_values(Some(os("")), Some(os("")), false);
    assert!(!unset.graphical && !empty.graphical);
    assert!(SessionFacts::from_values(Some(os(":0")), None, false).graphical);
    assert!(SessionFacts::from_values(None, Some(os("wayland-0")), false).graphical);
    assert!(SessionFacts::from_values(None, None, true).graphical);
    fn no_window(_: &ApprovalId) -> io::Result<()> {
        panic!("no window without a graphical session")
    }
    fn no_notice(_: &str, _: &str) {
        panic!("no notification without a graphical session")
    }
    let host = Host::new(&[(Capability::ExecTerminal, Ask)]);
    let headless = DesktopConsent::with(unset, no_window, no_notice);
    let principal = ProvenPrincipal::from(&host.authorized.principal);
    let probe = ApprovalView::new(
        &ApprovalId::unset(),
        &principal,
        Capability::ExecTerminal,
        &ApprovalSummary::default(),
        NOW,
    );
    assert_eq!(headless.show(&probe), Some(ApprovalSurface::CliOnly));
    host.service
        .set_notifier(Arc::new(DesktopConsent::with(unset, no_window, no_notice)));
    let login = ExecStart {
        command: ExecCommand::Login,
        terminal: Some(crate::share::ExecTerminalRequest {
            term: "xterm-256color".into(),
            size: crate::share::ExecTerminalSize { cols: 80, rows: 24 },
        }),
        ..shell("a9", "")
    };
    let (decided, frames, view) = drive(
        &host,
        &login,
        &client_features(),
        Act::Answer(ApprovalDecision::Deny),
    );
    assert_eq!(refused(decided).error.code, APPROVAL_DENIED);
    let pending = ExecApprovalPending::new(
        login.exec_id.clone(),
        Capability::ExecTerminal,
        ApprovalSurface::CliOnly,
    );
    assert_eq!(frames, [ServerFrame::ApprovalPending(pending)]);
    assert_eq!(view.surface.as_deref(), Some("cli_only"));
    assert!(approval_status_line("Host", Some(ApprovalSurface::CliOnly))
        .ends_with("(on the device only with `se share exec approvals`)"));
    // The request as `se share exec approvals --json` lists it (D.2.4).
    let listed = serde_json::to_value(&view).unwrap();
    let keys: Vec<&str> = listed
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for key in [
        "id",
        "device_name",
        "device_id",
        "fingerprint",
        "relation",
        "room",
        "capability",
        "account_label",
        "command",
        "requested_at",
        "surface",
    ] {
        assert!(keys.contains(&key), "{key} missing in {listed}");
    }
    assert_eq!(listed["capability"], "exec_terminal");
    assert_eq!(listed["command"], serde_json::json!({"mode": "login"}));
    assert!(listed.get("older_client_terminal").is_none());
    // The window program: the worker itself, or the one next to `se`.
    let gui = Path::new("/opt/se-app/smart_explorer");
    assert_eq!(gui_program(gui, |_| false).as_deref(), Some(gui));
    let name = if cfg!(windows) {
        "Smart Explorer.exe"
    } else {
        "smart_explorer"
    };
    let found = gui_program(Path::new("/opt/se-app/se"), |path| path.ends_with(name));
    assert_eq!(found, Some(Path::new("/opt/se-app").join(name)));
}
