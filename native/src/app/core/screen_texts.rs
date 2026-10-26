//! The German texts of the desktop viewer (SCR-M7, U25-9): states of a
//! session, error codes of the host, reasons a session ended and reasons
//! input does not reach the host. Every state, every `ScreenErrorCode` and
//! every `EndReason` has its own text (`screen_texts_tests.rs`); a code of
//! a newer host stays visible as „Fehler des Geräts: {Code}“.
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::ScreenErrorCode;

/// `base` with `: detail` when the host gave a diagnostic text.
fn with_detail(base: &str, detail: Option<&str>) -> String {
    match detail.map(str::trim).filter(|detail| !detail.is_empty()) {
        Some(detail) => format!("{base}: {detail}"),
        None => base.to_string(),
    }
}

/// Why the host cannot capture (`no_display`), from `STATE.reason`.
fn no_display_reason(reason: &str) -> String {
    match reason {
        "pipewire_missing" => "PipeWire fehlt auf dem Gerät",
        "capture_unavailable" => "keine Aufnahme verfügbar",
        "window_capture_unsupported" => "Fensteraufnahme braucht Windows 10 ab Version 1903",
        "portal_denied" => "die Freigabe wurde am Gerät abgelehnt",
        "monitor_not_found" => "Bildschirm nicht gefunden",
        "window_not_found" => "Fenster nicht gefunden",
        "portal_selects_source" => "die Quelle wählt der Nutzer am Gerät",
        "source_identity_unavailable" => "Android bestätigt keine Paketidentität. Android-Systemauswahl öffnen und die Quelle am Gerät auswählen",
        "no_display" => "keine Anzeige",
        other => other,
    }
    .to_string()
}

/// The status line of a session in `state`; empty while it streams.
pub(crate) fn state_text(state: SessionState, reason: Option<&str>) -> String {
    match state {
        SessionState::Connecting => "Verbindung wird aufgebaut …".into(),
        SessionState::WaitingForApproval => "Wartet auf die Bestätigung am Gerät …".into(),
        SessionState::WaitingForPortal => {
            "Wartet auf die Auswahl im Systemdialog des Geräts …".into()
        }
        SessionState::Starting => "Anzeige startet …".into(),
        SessionState::Streaming => String::new(),
        SessionState::SecureDesktop => {
            "Windows zeigt den sicheren Desktop – Anzeige und Eingabe sind durch Windows gesperrt"
                .into()
        }
        SessionState::WindowMinimized => "Fenster ist minimiert".into(),
        SessionState::HostLocked => "Das Gerät ist gesperrt".into(),
        SessionState::SessionInactive => "Die Benutzersitzung des Geräts ist nicht aktiv".into(),
        SessionState::NoDisplay => {
            let base = "Das Gerät kann seinen Bildschirm nicht aufnehmen";
            match reason.filter(|reason| !reason.is_empty()) {
                Some(reason) => format!("{base}: {}", no_display_reason(reason)),
                None => base.into(),
            }
        }
        SessionState::Ended => match reason.filter(|reason| !reason.is_empty()) {
            Some(reason) => end_reason_text(reason, None),
            None => "Sitzung beendet".into(),
        },
        SessionState::Unknown => "Unbekannter Zustand des Geräts".into(),
    }
}

/// The text of an error code of the host (`HELLO_ERROR`, `ERROR`).
pub(crate) fn error_text(code: &str, detail: Option<&str>) -> String {
    let Some(known) = ScreenErrorCode::from_key(code) else {
        return format!("Fehler des Geräts: {code}");
    };
    match known {
        ScreenErrorCode::PermissionDenied => "Das Gerät erlaubt diesen Zugriff nicht".into(),
        ScreenErrorCode::RightNotGranted => {
            "Dieses Gerät darf den Bildschirm nur ansehen, nicht steuern".into()
        }
        ScreenErrorCode::ApprovalDenied => "Der Nutzer am Gerät hat abgelehnt".into(),
        ScreenErrorCode::ApprovalCancelled => "Die Anfrage wurde abgebrochen".into(),
        ScreenErrorCode::NoDisplay => "Das Gerät hat keinen Bildschirm für die Anzeige".into(),
        ScreenErrorCode::IndicatorUnavailable => {
            "Bildschirmfreigabe braucht die Desktop-App auf dem Gerät".into()
        }
        ScreenErrorCode::ControlBusy => {
            "Das Gerät wird schon von einem anderen Gerät gesteuert".into()
        }
        ScreenErrorCode::ControlUnavailable => "Dieses Gerät lässt sich hier nicht steuern".into(),
        ScreenErrorCode::HostShutdown => "Das Gerät hat die Freigabe beendet".into(),
        ScreenErrorCode::ScreenUnsupported => {
            "Das Gerät kennt die Bildschirmfreigabe nicht (ältere Version)".into()
        }
        ScreenErrorCode::ApprovalUnavailable => "Am Gerät kann niemand gefragt werden".into(),
        ScreenErrorCode::CaptureFailed => {
            with_detail("Die Aufnahme am Gerät ist fehlgeschlagen", detail)
        }
        ScreenErrorCode::EncoderFailed => {
            with_detail("Die Kodierung am Gerät ist fehlgeschlagen", detail)
        }
        ScreenErrorCode::SessionLimit => {
            "Das Gerät bedient schon die höchstmögliche Zahl von Ansichten".into()
        }
    }
}

/// The text of a reason key of `EndReason` (and of the relay itself).
fn end_reason_text(key: &str, detail: Option<&str>) -> String {
    match key {
        "viewer_closed" | "ended" => "Sitzung beendet".into(),
        "host_stopped" => "Das Gerät hat die Ansicht beendet".into(),
        "capability_narrowed" => "Das Recht für diese Ansicht wurde entzogen".into(),
        "grant_expired" => "Das Recht für diese Ansicht ist abgelaufen".into(),
        "update_stop" => "Das Gerät wird aktualisiert".into(),
        "connection_lost" => "Die Verbindung zum Gerät ist abgebrochen".into(),
        "connection_failed" => with_detail("Die Verbindung zum Gerät ist fehlgeschlagen", detail),
        "failed" => with_detail("Die Sitzung ist fehlgeschlagen", detail),
        other => error_text(other, detail),
    }
}

/// The text of the end of a relayed session (`RelayEnd.code` and `.detail`;
/// the daemon module keeps that type private): a reason key of `EndReason`
/// or an error code of the host (`host_shutdown` is both).
pub(crate) fn end_text(code: &str, detail: Option<&str>) -> String {
    end_reason_text(code, detail)
}

/// Why input does not reach the host.
pub(crate) fn blocked_text(block: ControlBlock) -> &'static str {
    match block {
        ControlBlock::ElevatedWindow => {
            "Das Fenster am Gerät läuft mit höheren Rechten; Eingaben sind blockiert"
        }
        ControlBlock::SecureDesktop => "Windows zeigt den sicheren Desktop; Eingaben sind gesperrt",
        ControlBlock::ControlUnavailable => "Dieses Gerät erlaubt hier keine Fernsteuerung",
        ControlBlock::UnsupportedOnHost => "Dieses Gerät kann noch keine Eingaben annehmen",
        ControlBlock::Other => "Eingaben sind blockiert",
    }
}

#[cfg(test)]
#[path = "screen_texts_tests.rs"]
mod tests;
