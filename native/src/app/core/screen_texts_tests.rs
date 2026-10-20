//! `screen_task_viewer_state_texts_complete` (SCR-M7, U25-14): every state of
//! a session, every error code of the host, every reason a session ends and
//! every reason input is blocked has a German text of its own.
use super::*;
use crate::screen::core::session_state::EndReason;

const STATES: [SessionState; 12] = [
    SessionState::Connecting,
    SessionState::WaitingForApproval,
    SessionState::WaitingForPortal,
    SessionState::Starting,
    SessionState::Streaming,
    SessionState::SecureDesktop,
    SessionState::WindowMinimized,
    SessionState::HostLocked,
    SessionState::SessionInactive,
    SessionState::NoDisplay,
    SessionState::Ended,
    SessionState::Unknown,
];

const END_REASONS: [EndReason; 8] = [
    EndReason::ViewerClosed,
    EndReason::HostStopped,
    EndReason::CapabilityNarrowed,
    EndReason::GrantExpired,
    EndReason::HostShutdown,
    EndReason::UpdateStop,
    EndReason::ConnectionLost,
    EndReason::Failed,
];

const BLOCKS: [ControlBlock; 5] = [
    ControlBlock::ElevatedWindow,
    ControlBlock::SecureDesktop,
    ControlBlock::ControlUnavailable,
    ControlBlock::UnsupportedOnHost,
    ControlBlock::Other,
];

#[test]
fn screen_task_viewer_state_texts_complete() {
    // Every state has a text; only `streaming` has none.
    let mut seen = Vec::new();
    for state in STATES {
        let text = state_text(state, None);
        assert_eq!(
            text.is_empty(),
            state == SessionState::Streaming,
            "{}: {text:?}",
            state.key()
        );
        assert!(
            !seen.contains(&text) || text.is_empty(),
            "two states share {text:?}"
        );
        seen.push(text);
    }
    assert_eq!(
        state_text(SessionState::WaitingForPortal, None),
        "Wartet auf die Auswahl im Systemdialog des Geräts …"
    );
    assert_eq!(
        state_text(SessionState::NoDisplay, Some("pipewire_missing")),
        "Das Gerät kann seinen Bildschirm nicht aufnehmen: PipeWire fehlt auf dem Gerät"
    );
    assert_eq!(
        state_text(SessionState::NoDisplay, Some("neuer_grund")),
        "Das Gerät kann seinen Bildschirm nicht aufnehmen: neuer_grund"
    );
    assert_eq!(
        state_text(SessionState::Ended, Some("host_stopped")),
        "Das Gerät hat die Ansicht beendet"
    );

    // Every error code of the host has its own text, none the fallback.
    let mut texts = Vec::new();
    for code in ScreenErrorCode::ALL {
        let text = error_text(code.key(), None);
        assert!(!text.starts_with("Fehler des Geräts"), "{}", code.key());
        assert!(!texts.contains(&text), "{} repeats {text:?}", code.key());
        texts.push(text);
    }
    assert_eq!(
        error_text("capture_failed", Some("kein Bild")),
        "Die Aufnahme am Gerät ist fehlgeschlagen: kein Bild"
    );
    assert_eq!(
        error_text("encoder_failed", None),
        "Die Kodierung am Gerät ist fehlgeschlagen"
    );
    assert_eq!(
        error_text("von_einem_neuen_geraet", None),
        "Fehler des Geräts: von_einem_neuen_geraet"
    );

    // Every reason a session ends has a text, whether the relay names it
    // by an `EndReason` or by the code of a refusal.
    for reason in END_REASONS {
        let text = end_text(reason.key(), None);
        assert!(!text.starts_with("Fehler des Geräts"), "{}", reason.key());
        assert!(!text.is_empty());
    }
    for code in ScreenErrorCode::ALL {
        assert!(!end_text(code.key(), None).starts_with("Fehler des Geräts"));
    }
    assert_eq!(
        end_text("failed", Some("Zeitüberschreitung")),
        "Die Sitzung ist fehlgeschlagen: Zeitüberschreitung"
    );
    assert_eq!(end_text("ended", None), "Sitzung beendet");
    assert_eq!(
        end_text("connection_lost", Some("egal")),
        "Die Verbindung zum Gerät ist abgebrochen"
    );
    assert_eq!(
        end_text("approval_denied", None),
        error_text("approval_denied", None)
    );

    // Every block has a text of its own.
    let blocked: Vec<&str> = BLOCKS.iter().map(|block| blocked_text(*block)).collect();
    for (index, text) in blocked.iter().enumerate() {
        assert!(!text.is_empty());
        assert!(!blocked[..index].contains(text), "{text:?} repeats");
    }
}
