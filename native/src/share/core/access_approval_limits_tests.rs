//! PRM-M3, second part: the reservations of open requests (Exec and screen),
//! a grant narrowed while its request is open, and the `cli_only` surface of
//! a host without a graphical session. The host fixture is in
//! `access_approval_tests.rs`.
use std::io;
use std::path::Path;

use super::*;
use crate::share::access_approval::DEVICE_RESERVATIONS;
use crate::share::access_grant::CapabilityDecision::Off;
use crate::share::consent_launch::{gui_program, DesktopConsent, SessionFacts};
use crate::share::exec_client_messages::approval_status_line;

#[test]
fn perm_task_ask_counts_against_peer_limit() {
    struct Jobs(usize, usize);
    impl ReservationGauge for Jobs {
        fn active(&self, _device_id: &str) -> (usize, usize) {
            (self.0, self.1)
        }
    }
    let host = Host::new(&[(Capability::ExecCommands, Ask)]);
    let device = |id: &str| ProvenPrincipal {
        device_id: id.into(),
        ..ProvenPrincipal::from(&host.authorized.principal)
    };
    let service = &host.service;
    let summary = ApprovalSummary::default;
    let first = service.request(
        &Jobs(0, 0),
        &device("x"),
        Capability::ExecCommands,
        summary(),
    );
    let second = service.request(
        &Jobs(0, 0),
        &device("x"),
        Capability::ExecTerminal,
        summary(),
    );
    assert!(first.surface().is_some() && second.surface().is_some());
    assert_eq!(DEVICE_RESERVATIONS, 2);
    let mut third = service.request(
        &Jobs(0, 0),
        &device("x"),
        Capability::ExecCommands,
        summary(),
    );
    assert_eq!(run(third.outcome()), ApprovalOutcome::DeviceLimit);
    // An allowed start of the device does not fit either; another device does.
    assert_eq!(
        service.admits(&Jobs(0, 0), "x"),
        Err(ApprovalOutcome::DeviceLimit)
    );
    assert!(service.admits(&Jobs(0, 0), "y").is_ok());
    let error = limit_error(ApprovalOutcome::DeviceLimit);
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("admission_denied", "PrincipalLimit")
    );
    // One open request and one running job are two reservations as well.
    assert!(first.cancel());
    assert_eq!(
        service.admits(&Jobs(1, 1), "x"),
        Err(ApprovalOutcome::DeviceLimit)
    );
    assert!(service.admits(&Jobs(0, 1), "x").is_ok());
    // All eight of the host: one open request and seven jobs elsewhere.
    let mut late = service.request(
        &Jobs(0, 7),
        &device("z"),
        Capability::ExecCommands,
        summary(),
    );
    assert_eq!(run(late.outcome()), ApprovalOutcome::HostLimit);
    assert_eq!(
        limit_error(ApprovalOutcome::HostLimit).message,
        "GlobalLimit"
    );
    drop(second);
    assert!(host.service.pending().is_empty());
    // End to end: a running session and an open request of the device leave
    // no room for a start that asks.
    host.running(&shell("e0", "sleep 60"));
    let held = service.request(
        host.registry.as_ref(),
        &device(host.device()),
        Capability::ExecCommands,
        summary(),
    );
    assert!(held.surface().is_some());
    let decided = run(async {
        let (_client, server) = tokio::io::duplex(64 * 1024);
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut stream = ClientStream::new(client_frames(server_read));
        let start = shell("e1", "ls");
        let mut protocol = ServerProtocolState::default();
        let sink = |entry: AccessEntry<'_>| host.log.lock().unwrap().push(access_line(&entry));
        let approval = StartApproval {
            service,
            gauge: host.registry.as_ref(),
            auth: &host.auth,
            authorized: &host.authorized,
            start: &start,
            host: "Host",
            client_features: &[],
            own_label: "alice",
            audit: &sink,
            grant_poll: Duration::from_millis(50),
        };
        decide_start(
            &approval,
            &mut server_write,
            &mut stream,
            &mut protocol,
            POLICY,
        )
        .await
        .unwrap()
    });
    let refusal = refused(decided);
    assert_eq!(refusal.error.message, "PrincipalLimit");
    assert_eq!(host.events(), ["approval_unavailable"]);
}

#[test]
fn perm_task_screen_approval_counts_against_exec_reservations() {
    let host = Host::new(&[(Capability::ExecCommands, Allow)]);
    host.running(&shell("f0", "sleep 60"));
    // S4 asks through the broker: `request(principal, capability, summary)`.
    let broker = host.service.broker(host.registry.clone());
    let principal = ProvenPrincipal::from(&host.authorized.principal);
    let summary = ApprovalSummary {
        device_name: Some("Testgerät".into()),
        ..ApprovalSummary::default()
    };
    let mut screen = broker.request(&principal, Capability::ScreenView, summary.clone());
    assert_eq!(screen.surface(), Some(ApprovalSurface::Window));
    let open = host.service.pending();
    assert_eq!(
        (open[0].capability, open[0].command.is_none()),
        (Capability::ScreenView, true)
    );
    // The running session and the screen request take both reservations.
    let full = host.service.admits(host.registry.as_ref(), host.device());
    assert_eq!(full, Err(ApprovalOutcome::DeviceLimit));
    let mut more = broker.request(&principal, Capability::ScreenControl, summary);
    assert_eq!(run(more.outcome()), ApprovalOutcome::DeviceLimit);
    // The answer frees the screen reservation; an Exec start fits again.
    let id = screen.id().clone();
    host.service.decide(&id, ApprovalDecision::Once).unwrap();
    assert_eq!(run(screen.outcome()), ApprovalOutcome::Once);
    assert!(host
        .service
        .admits(host.registry.as_ref(), host.device())
        .is_ok());
    let admission = host.registry.prepare(
        host.authorized.principal.clone(),
        host.authorized.authorization.clone(),
        &shell("f1", "ls"),
        NOW,
    );
    assert!(matches!(admission, Ok(ExecAdmission::Prepared(_))));
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
