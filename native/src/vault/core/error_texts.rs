//! The display texts of the structured vault codes (vault plan C3.2, C7.4,
//! U26-8): every surface that gets `Err.code` from the worker shows the same
//! words. Pure.

/// Every code a vault request answers with (`IpcResponse::Err.code`, the
/// bridge's `ApiError` kind), in the order of the plan.
pub(crate) const CODES: [&str; 10] = [
    "vault_locked",
    "vault_wrong_password",
    "vault_unsupported_format",
    "vault_hub",
    "vault_memory",
    "vault_no_saved_password",
    "vault_unsaved_changes",
    "vault_not_atomic",
    "compare_stale",
    "compare_too_large",
];

/// The code of a comparison that is gone (service restarted, newer
/// comparison of the same job, lock): not a vault code in the narrow sense.
pub(crate) const COMPARE_STALE_CODE: &str = "compare_stale";
/// The code of an answer (the two versions of a text) that does not fit one
/// line of the service.
pub(crate) const COMPARE_TOO_LARGE_CODE: &str = "compare_too_large";

/// The text for `code`; `None` for a code this build does not know.
pub(crate) fn error_text_for_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "vault_locked" => "Der Tresor ist gesperrt. Entsperren mit `se vault unlock <Tresor>`.",
        "vault_wrong_password" => "Das Passwort ist falsch.",
        "vault_unsupported_format" => "Dieses Tresorformat wird nicht unterstützt.",
        "vault_hub" => {
            "Dieser Tresor gehört zu einem Cryptomator Hub und lässt sich nicht mit einem \
             Passwort entsperren."
        }
        "vault_memory" => "Der Arbeitsspeicher reicht zum Entsperren dieses Tresors nicht.",
        "vault_no_saved_password" => "Für diesen Tresor ist kein Passwort gespeichert.",
        "vault_unsaved_changes" => "Es gibt noch nicht gespeicherte Änderungen in diesem Tresor.",
        "vault_not_atomic" => {
            "Der Speicherort kann Dateien nicht atomar ersetzen; ein Passwortwechsel ist dort \
             nicht sicher."
        }
        "compare_stale" => "Der Vergleich ist veraltet. Bitte erneut vergleichen.",
        "compare_too_large" => "Die Datei ist zu groß für die Zusammenführung.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_task_error_texts_cover_every_code() {
        for code in CODES {
            let text = error_text_for_code(code).unwrap_or_else(|| panic!("text of {code}"));
            assert!(!text.trim().is_empty(), "{code}");
            assert!(text.ends_with('.'), "{code}: {text}");
        }
        assert_eq!(error_text_for_code("vault_unbekannt"), None);
        assert!(error_text_for_code("vault_locked")
            .unwrap_or_default()
            .contains("se vault unlock"));
        assert_eq!(CODES.len(), 10);
        assert!(CODES.contains(&COMPARE_STALE_CODE));
        assert!(CODES.contains(&COMPARE_TOO_LARGE_CODE));
        // The code a wrong password gives is the one `UnlockError::code` uses.
        assert!(CODES.contains(&"vault_wrong_password"));
    }
}
