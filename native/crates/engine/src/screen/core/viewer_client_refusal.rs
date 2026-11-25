//! The texts of a host's refusal (moved out of `viewer_client.rs`).
use super::super::wire_hello::ScreenErrorCode;

/// The code key of a refusal text (`encoder_failed: chosen_codec_…` →
/// `encoder_failed`), so code comparisons keep working with a reason.
pub(super) fn refusal_key(text: &str) -> &str {
    text.split_once(": ").map_or(text, |(key, _)| key)
}

/// The German text of a refusal code; unknown codes of newer hosts stay as
/// they are.
pub fn refusal_text(code: &str) -> String {
    let text = match ScreenErrorCode::from_key(code) {
        Some(ScreenErrorCode::PermissionDenied) => {
            "Dieses Gerät darf den Bildschirm nicht ansehen (Recht „Bildschirm ansehen“ ist aus)"
        }
        Some(ScreenErrorCode::RightNotGranted) => {
            "Nur Ansehen ist erlaubt; „Bildschirm steuern“ ist aus"
        }
        Some(ScreenErrorCode::ApprovalDenied) => "Am Gerät abgelehnt",
        Some(ScreenErrorCode::ApprovalCancelled) => "Die Rückfrage wurde zurückgezogen",
        Some(ScreenErrorCode::ApprovalUnavailable) => "Am Gerät kann niemand gefragt werden",
        Some(ScreenErrorCode::NoDisplay) => "Das Gerät hat keine grafische Sitzung zum Zeigen",
        Some(ScreenErrorCode::IndicatorUnavailable) => {
            "Bildschirmfreigabe braucht die Desktop-App auf dem Gerät"
        }
        Some(ScreenErrorCode::ControlBusy) => "Ein anderes Gerät steuert gerade",
        Some(ScreenErrorCode::ControlUnavailable) => "Steuern ist auf diesem Gerät nicht möglich",
        Some(ScreenErrorCode::HostShutdown) => "Smart Explorer auf dem Gerät wurde beendet",
        Some(ScreenErrorCode::ScreenUnsupported) => {
            "Gerät kennt die Bildschirmfreigabe nicht (ältere Version)"
        }
        Some(ScreenErrorCode::CaptureFailed) => "Die Aufnahme auf dem Gerät ist fehlgeschlagen",
        Some(ScreenErrorCode::EncoderFailed) => "Der Kodierer auf dem Gerät ist fehlgeschlagen",
        Some(ScreenErrorCode::SessionLimit) => {
            "Das Gerät zeigt schon die höchste Zahl an Sitzungen"
        }
        None => return code.to_string(),
    };
    text.to_string()
}
