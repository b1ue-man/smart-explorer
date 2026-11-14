//! Tests of E28 (plan p-zugang, M9): open requests are not limited (R15); the
//! requests of one device share one shown window or notification; each request
//! is decided on its own; equal requests share one answer. The answer reports
//! how many other requests it answered too (M32).
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::{
    ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalOutcome, ApprovalService,
    ApprovalSummary, ApprovalTicket, ApprovalView, ConsentNotifier,
};
use crate::share::access_grant::Capability;
use crate::share::exec_access_view::ApprovalSurface;
use crate::share::relation_proof::ProvenPrincipal;

/// Shows each request on `surface` and counts the shows: one window or one
/// notification each.
struct Shows {
    surface: Option<ApprovalSurface>,
    count: AtomicUsize,
}

impl ConsentNotifier for Shows {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.surface
    }
}

fn service(surface: Option<ApprovalSurface>) -> (ApprovalService, Arc<Shows>) {
    let shows = Arc::new(Shows {
        surface,
        count: AtomicUsize::new(0),
    });
    (ApprovalService::new(shows.clone()), shows)
}

fn shown(shows: &Shows) -> usize {
    shows.count.load(Ordering::SeqCst)
}

fn principal(device: &str) -> ProvenPrincipal {
    ProvenPrincipal {
        relation_kind: "direct".into(),
        relation_id: String::new(),
        device_id: device.into(),
        device_name: format!("Name {device}"),
        public_key: format!("key-{device}"),
        fingerprint: format!("fp-{device}"),
        node_id: format!("key-{device}"),
    }
}

fn summary(command: &str, revision: u64) -> ApprovalSummary {
    ApprovalSummary {
        account_label: Some("alice".into()),
        command: Some(ApprovalCommand::Shell {
            text: command.into(),
        }),
        policy_revision: revision,
        ..ApprovalSummary::default()
    }
}

fn ask(service: &ApprovalService, device: &str, command: &str) -> ApprovalTicket {
    service.request(
        &principal(device),
        Capability::ExecCommands,
        summary(command, 3),
    )
}

fn open_of(service: &ApprovalService, device: &str) -> Vec<ApprovalId> {
    service
        .pending()
        .into_iter()
        .filter(|view| view.device_id == device)
        .map(|view| view.id)
        .collect()
}

fn outcome(ticket: &mut ApprovalTicket) -> ApprovalOutcome {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(ticket.outcome())
}

#[test]
fn rueckfragen_task_ten_requests_of_one_device_show_one_window() {
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let tickets: Vec<ApprovalTicket> = (0..10)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    assert_eq!(
        shown(&shows),
        1,
        "one window and one notification per device"
    );
    assert_eq!(open_of(&service, "device-a").len(), 10, "no limit (R15)");
    assert!(tickets
        .iter()
        .all(|ticket| matches!(ticket.surface(), Some(ApprovalSurface::Window))));
}

#[test]
fn rueckfragen_task_each_request_of_a_device_is_decided_alone() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> = (0..10)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    let mut open = 10;
    for ticket in tickets.iter_mut() {
        let id = ticket.id().clone();
        service.decide(&id, ApprovalDecision::Once).unwrap();
        open -= 1;
        assert_eq!(outcome(ticket), ApprovalOutcome::Once);
        assert_eq!(open_of(&service, "device-a").len(), open);
    }
}

#[test]
fn rueckfragen_task_equal_requests_share_one_always_answer() {
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> =
        (0..10).map(|_| ask(&service, "device-a", "same")).collect();
    let first = open_of(&service, "device-a")[0].clone();
    service.decide(&first, ApprovalDecision::Always).unwrap();
    assert!(open_of(&service, "device-a").is_empty());
    assert_eq!(shown(&shows), 1);
    for ticket in tickets.iter_mut() {
        assert_eq!(outcome(ticket), ApprovalOutcome::Always);
    }
}

#[test]
fn rueckfragen_task_equal_requests_share_one_denial() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> =
        (0..10).map(|_| ask(&service, "device-a", "same")).collect();
    let first = open_of(&service, "device-a")[0].clone();
    service.decide(&first, ApprovalDecision::Deny).unwrap();
    assert!(open_of(&service, "device-a").is_empty());
    for ticket in tickets.iter_mut() {
        assert_eq!(outcome(ticket), ApprovalOutcome::Denied);
    }
}

#[test]
fn rueckfragen_task_once_answers_only_the_chosen_equal_request() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> =
        (0..3).map(|_| ask(&service, "device-a", "same")).collect();
    let chosen = open_of(&service, "device-a")[0].clone();
    service.decide(&chosen, ApprovalDecision::Once).unwrap();
    assert_eq!(open_of(&service, "device-a").len(), 2);
    let ticket = tickets
        .iter_mut()
        .find(|ticket| ticket.id() == &chosen)
        .unwrap();
    assert_eq!(outcome(ticket), ApprovalOutcome::Once);
}

#[test]
fn rueckfragen_task_other_grant_revision_is_no_equal_request() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let older = service.request(
        &principal("device-a"),
        Capability::ExecCommands,
        summary("same", 3),
    );
    let _newer = service.request(
        &principal("device-a"),
        Capability::ExecCommands,
        summary("same", 4),
    );
    service
        .decide(older.id(), ApprovalDecision::Always)
        .unwrap();
    assert_eq!(open_of(&service, "device-a").len(), 1);
}

#[test]
fn rueckfragen_task_requests_of_two_devices_show_two_windows() {
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let _first: Vec<ApprovalTicket> = (0..3)
        .map(|number| ask(&service, "device-a", &format!("a {number}")))
        .collect();
    let _second: Vec<ApprovalTicket> = (0..2)
        .map(|number| ask(&service, "device-b", &format!("b {number}")))
        .collect();
    assert_eq!(shown(&shows), 2);
    assert_eq!(open_of(&service, "device-a").len(), 3);
    assert_eq!(open_of(&service, "device-b").len(), 2);
}

#[test]
fn rueckfragen_task_ended_window_lets_the_next_request_open_a_new_one() {
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let tickets: Vec<ApprovalTicket> = (0..3)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    let window = tickets[0].id().clone();
    service.window_ended(&window);
    let later = ask(&service, "device-a", "later");
    assert_eq!(shown(&shows), 2, "a new window for the device");
    assert!(matches!(later.surface(), Some(ApprovalSurface::Window)));
}

#[test]
fn rueckfragen_task_answered_anchor_passes_the_window_to_the_next_request() {
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let tickets: Vec<ApprovalTicket> = (0..3)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    let anchor = tickets[0].id().clone();
    service.decide(&anchor, ApprovalDecision::Once).unwrap();
    assert_eq!(open_of(&service, "device-a").len(), 2);
    let later = ask(&service, "device-a", "later");
    assert_eq!(shown(&shows), 1, "the open window keeps showing the device");
    assert!(matches!(later.surface(), Some(ApprovalSurface::Window)));
}

#[test]
fn rueckfragen_task_withdrawn_anchor_passes_the_window_on() {
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> = (0..2)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    let anchor = tickets.remove(0);
    assert!(anchor.cancel());
    let _later = ask(&service, "device-a", "later");
    assert_eq!(shown(&shows), 1);
    assert_eq!(open_of(&service, "device-a").len(), 2);
}

#[test]
fn rueckfragen_task_answered_notified_request_passes_the_notification_on() {
    let (service, shows) = service(Some(ApprovalSurface::Notification));
    let tickets: Vec<ApprovalTicket> = (0..3)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    assert_eq!(shown(&shows), 1);
    let anchor = tickets[0].id().clone();
    service.decide(&anchor, ApprovalDecision::Once).unwrap();
    assert_eq!(
        shown(&shows),
        2,
        "the notification moves to an open request"
    );
    assert_eq!(open_of(&service, "device-a").len(), 2);
}

#[test]
fn rueckfragen_task_window_whose_own_request_ended_shows_the_rest_again() {
    // The window's own request ends before the window could list its device; a
    // request that joined meanwhile must not stay without a window.
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> = (0..2)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    let own = tickets.remove(0);
    let window = own.id().clone();
    assert!(own.cancel());
    assert_eq!(shown(&shows), 1, "the open window keeps the device");
    service.window_ended(&window);
    assert_eq!(shown(&shows), 2, "the rest is shown again once");
    let joined = &service.pending()[0];
    assert_eq!(&joined.id, tickets[0].id());
    assert_eq!(joined.surface.as_deref(), Some("window"));
    let later = ask(&service, "device-a", "later");
    assert_eq!(shown(&shows), 2, "later requests join the new window");
    assert!(matches!(later.surface(), Some(ApprovalSurface::Window)));
}

#[test]
fn rueckfragen_task_window_ended_with_its_own_request_open_shows_nothing_again() {
    // A crash or an update stop ends the window without an answer: no new
    // window opens by itself (as before E28).
    let (service, shows) = service(Some(ApprovalSurface::Window));
    let tickets: Vec<ApprovalTicket> = (0..2)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    service.window_ended(tickets[0].id());
    assert_eq!(shown(&shows), 1);
    assert_eq!(open_of(&service, "device-a").len(), 2);
}

#[test]
fn rueckfragen_task_cli_only_request_lets_the_next_one_ask_again() {
    // `cli_only` shows nothing; a graphical session may appear later, so the
    // next request of the device is shown on its own.
    let (service, shows) = service(Some(ApprovalSurface::CliOnly));
    let _tickets: Vec<ApprovalTicket> = (0..3)
        .map(|number| ask(&service, "device-a", &format!("command {number}")))
        .collect();
    assert_eq!(shown(&shows), 3);
    assert_eq!(open_of(&service, "device-a").len(), 3);
}

#[test]
fn rueckfragen_task_without_a_surface_nobody_is_asked_and_nothing_joins() {
    let (service, shows) = service(None);
    let mut first = ask(&service, "device-a", "one");
    assert_eq!(outcome(&mut first), ApprovalOutcome::Unavailable);
    let mut second = ask(&service, "device-a", "two");
    assert_eq!(outcome(&mut second), ApprovalOutcome::Unavailable);
    assert_eq!(shown(&shows), 2, "each request asks again");
    assert!(open_of(&service, "device-a").is_empty());
}

#[test]
fn rueckfragen_task_always_reports_the_other_equal_requests_it_answered() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> =
        (0..10).map(|_| ask(&service, "device-a", "same")).collect();
    let first = open_of(&service, "device-a")[0].clone();
    assert_eq!(service.decide(&first, ApprovalDecision::Always).unwrap(), 9);
    for ticket in tickets.iter_mut() {
        assert_eq!(outcome(ticket), ApprovalOutcome::Always);
    }
}

#[test]
fn rueckfragen_task_denial_reports_the_other_equal_requests_it_answered() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let mut tickets: Vec<ApprovalTicket> =
        (0..10).map(|_| ask(&service, "device-a", "same")).collect();
    let first = open_of(&service, "device-a")[0].clone();
    assert_eq!(service.decide(&first, ApprovalDecision::Deny).unwrap(), 9);
    for ticket in tickets.iter_mut() {
        assert_eq!(outcome(ticket), ApprovalOutcome::Denied);
    }
}

#[test]
fn rueckfragen_task_once_reports_no_other_request_as_answered() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let _tickets: Vec<ApprovalTicket> = (0..3).map(|_| ask(&service, "device-a", "same")).collect();
    let chosen = open_of(&service, "device-a")[0].clone();
    assert_eq!(service.decide(&chosen, ApprovalDecision::Once).unwrap(), 0);
    assert_eq!(open_of(&service, "device-a").len(), 2);
}

#[test]
fn rueckfragen_task_only_equal_requests_count_as_answered() {
    let (service, _shows) = service(Some(ApprovalSurface::Window));
    let same: Vec<ApprovalTicket> = (0..2).map(|_| ask(&service, "device-a", "same")).collect();
    let _other = ask(&service, "device-a", "other");
    let chosen = same[0].id().clone();
    assert_eq!(service.decide(&chosen, ApprovalDecision::Deny).unwrap(), 1);
    assert_eq!(
        open_of(&service, "device-a").len(),
        1,
        "the request with another command stays open"
    );
}
