//! The daemon side of a worker without session variables (E23, S3, second
//! run): its X11 connections reach the followed session only through a
//! verified display, server and cookie; its session kind and its learned
//! facts follow a switch; the processes it starts across the account
//! boundary are marked and held; the consent window asks in the followed
//! session. A worker with own session variables keeps its paths.
use std::cell::Cell;

use super::backend::{Follow, PerSession};
use super::process::{is_started_host, marker, seals, window_closed_by_person, window_moved};
use super::x11::{
    parse_display, pick_cookie, route, server_allowed, LocalDisplay, Route, COOKIE_NAME,
    FAMILY_LOCAL, FAMILY_WILD,
};
use crate::screen::os::session_detect::SessionKind;
use crate::share::consent_launch::{asks_in_window, SessionFacts};

/// One Xauthority entry in the file format of libXau.
fn entry(family: u16, address: &[u8], number: &str, name: &[u8], data: &[u8]) -> Vec<u8> {
    let mut bytes = family.to_be_bytes().to_vec();
    for field in [address, number.as_bytes(), name, data] {
        bytes.extend_from_slice(&(field.len() as u16).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    bytes
}

#[test]
fn bildschirm_sitzung_task_daemon_x11_connects_to_the_followed_display() {
    // Own session variables: the unchanged connection through DISPLAY.
    assert_eq!(route(true, None), Route::Own);
    assert_eq!(route(true, Some(SessionKind::Wayland)), Route::Own);
    // A worker without them reaches only a followed X11 session.
    assert_eq!(route(false, Some(SessionKind::X11)), Route::Followed);
    assert!(matches!(
        route(false, Some(SessionKind::Wayland)),
        Route::Refused(_)
    ));
    assert!(matches!(route(false, None), Route::Refused(_)));
    assert_eq!(
        parse_display(":0"),
        Some(LocalDisplay {
            number: 0,
            screen: 0
        })
    );
    assert_eq!(
        parse_display(":12.1"),
        Some(LocalDisplay {
            number: 12,
            screen: 1
        })
    );
    for remote_or_malformed in ["", ":", "host:0", "localhost:10.0", ":0.", ":0.1.2", ":a"] {
        assert_eq!(
            parse_display(remote_or_malformed),
            None,
            "{remote_or_malformed}"
        );
    }
}

#[test]
fn bildschirm_sitzung_task_xauthority_cookie_is_picked_for_the_display() {
    let host = b"desk";
    let other_host = entry(FAMILY_LOCAL, b"other", "0", COOKIE_NAME, b"foreign");
    let other_number = entry(FAMILY_LOCAL, host, "1", COOKIE_NAME, b"display-one");
    let other_name = entry(FAMILY_LOCAL, host, "0", b"XDM-AUTHORIZATION-1", b"xdm");
    let wanted = entry(FAMILY_LOCAL, host, "0", COOKIE_NAME, b"cookie");
    let file = [other_host.clone(), other_number.clone(), other_name, wanted].concat();
    assert_eq!(pick_cookie(&file, host, 0), Some(b"cookie".to_vec()));
    assert_eq!(pick_cookie(&file, host, 1), Some(b"display-one".to_vec()));
    assert_eq!(pick_cookie(&file, host, 2), None);
    // Wild entries and entries without a number match every display.
    let wild = entry(FAMILY_WILD, b"", "0", COOKIE_NAME, b"wild");
    assert_eq!(pick_cookie(&wild, b"any", 0), Some(b"wild".to_vec()));
    let any_number = entry(FAMILY_LOCAL, host, "", COOKIE_NAME, b"every");
    assert_eq!(pick_cookie(&any_number, host, 7), Some(b"every".to_vec()));
    // Another host's cookie is never sent.
    assert_eq!(pick_cookie(&other_host, host, 0), None);
    // A truncated file gives no cookie.
    let truncated = &other_number[..other_number.len() - 3];
    assert_eq!(pick_cookie(truncated, host, 1), None);
    assert_eq!(pick_cookie(&[], host, 0), None);
}

#[test]
fn bildschirm_sitzung_task_x_server_must_be_root_or_the_session_account() {
    assert!(server_allowed(0, 1000));
    assert!(server_allowed(1000, 1000));
    assert!(!server_allowed(1001, 1000));
    assert!(!server_allowed(65534, 1000));
}

#[test]
fn bildschirm_sitzung_task_daemon_session_kind_follows_a_switch() {
    // A worker with own session variables keeps the kind of its start.
    let fixed = Follow::fixed();
    assert!(!fixed.follows());
    let asked = Cell::new(false);
    let kind = fixed.session_with(SessionKind::X11, || {
        asked.set(true);
        SessionKind::Wayland
    });
    assert_eq!(kind, SessionKind::X11);
    assert!(!asked.get(), "a fixed backend never asks logind");
    assert_eq!(fixed.generation(), None);
    // What a backend learned belongs to one session.
    let facts = PerSession::<u32>::default();
    let made = Cell::new(0);
    let make = |value: u32| {
        made.set(made.get() + 1);
        value
    };
    assert_eq!(facts.get(None, || make(1)), 1);
    assert_eq!(
        facts.get(None, || make(2)),
        1,
        "once per process when fixed"
    );
    let switched = PerSession::<&str>::default();
    assert_eq!(switched.get(Some("c1:10:1000:7".into()), || "user"), "user");
    assert_eq!(
        switched.get(Some("c1:10:1000:7".into()), || "again"),
        "user"
    );
    assert_eq!(
        switched.get(Some("c2:20:108:9".into()), || "greeter"),
        "greeter"
    );
    assert_eq!(switched.get(Some(String::new()), || "none"), "none");
}

#[test]
fn bildschirm_sitzung_task_started_host_is_marked_and_held() {
    // A worker with own session variables does not mark its host.
    assert_eq!(marker(true, Some(SessionKind::X11)), None);
    assert_eq!(marker(true, None), None);
    // A worker without them marks the kind it started the host for.
    assert_eq!(marker(false, Some(SessionKind::X11)), Some("x11"));
    assert_eq!(marker(false, Some(SessionKind::Wayland)), Some("wayland"));
    assert_eq!(marker(false, None), Some("none"));
    // A marked host never follows a session itself (no root host on a
    // desktop); an unmarked process follows as before.
    assert!(is_started_host(Some("none")));
    assert!(is_started_host(Some("x11")));
    assert!(!is_started_host(None));
    // Only an X11 host closes /proc access to its pipes; a Wayland host
    // stays inspectable for the portal.
    assert!(seals(Some("x11")));
    assert!(!seals(Some("wayland")));
    assert!(!seals(Some("none")));
    assert!(!seals(None));
    // The indicator window gives way when the active session changed.
    assert!(window_moved(Some("c1:10:1000:7"), Some("c2:20:108:9")));
    assert!(window_moved(None, Some("c2:20:108:9")));
    assert!(!window_moved(Some("c1:10:1000:7"), Some("c1:10:1000:7")));
    assert!(!window_moved(Some("c1:10:1000:7"), None));
}

#[test]
fn bildschirm_sitzung_task_closed_indicator_of_a_followed_session_ends_its_sessions() {
    // The window in the followed session runs as its account and may not
    // reach the worker: a normal close ends the sessions (C7).
    assert!(window_closed_by_person(true, Some(0)));
    // An update stop, a failure or a signal keep their handling.
    assert!(!window_closed_by_person(true, Some(75)));
    assert!(!window_closed_by_person(true, Some(1)));
    assert!(!window_closed_by_person(true, None));
    // A worker with own session variables ends them through its window as
    // before (restart while sessions run).
    assert!(!window_closed_by_person(false, Some(0)));
}

#[test]
fn bildschirm_sitzung_task_consent_window_asks_in_the_followed_session() {
    let headless = SessionFacts { graphical: false };
    let desktop = SessionFacts { graphical: true };
    // Own graphical session: a window as before, logind is not asked.
    let asked = Cell::new(false);
    assert!(asks_in_window(desktop, false, || {
        asked.set(true);
        false
    }));
    assert!(!asked.get());
    // A worker with own variables but no display stays cli_only.
    assert!(!asks_in_window(headless, false, || true));
    // A worker without session variables asks in the session it follows,
    // and only while it follows one.
    assert!(asks_in_window(headless, true, || true));
    assert!(!asks_in_window(headless, true, || false));
}
