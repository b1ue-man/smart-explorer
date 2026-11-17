//! The update helper's last failure in words a person can act on: what went
//! wrong and what happens next on its own. Pure: the caller reads the failure
//! record (`last_updater_error.txt`) and the unfinished update transaction of
//! the installation. The app's update page, `se update` and `se update
//! --check` show the same texts.
use serde::Serialize;

/// An unfinished update transaction of the installation, by what its
/// installed files allow (`mcp_launcher::inspect_interrupted_update`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum PendingUpdate {
    /// Its updater still holds it.
    Running { target_version: String },
    /// The files are the previous version: the next update run rolls it back.
    RollBack {
        previous_version: String,
        target_version: String,
    },
    /// The files are the target version: the next update run completes it.
    Complete {
        previous_version: String,
        target_version: String,
    },
    /// The files are neither version (`detail` names each one).
    Refused {
        previous_version: String,
        target_version: String,
        detail: String,
    },
    /// The installation's update state could not be read.
    Unreadable { detail: String },
}

/// A failed or unfinished update as shown to the person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateFailure {
    /// The helper's own report; `None` when only an unfinished update is known.
    pub error: Option<String>,
    /// What went wrong, in German.
    pub reason: String,
    /// What happens next without anyone doing anything, in German.
    pub next_action: String,
    pub pending: Option<PendingUpdate>,
}

/// Marker in the helper's report → reason. The first marker found decides,
/// so the codes of the resumed transaction come before the general ones.
const REASONS: &[(&str, &str)] = &[
    ("update_recovery_refused", "Eine unterbrochene Aktualisierung hat die Programmdateien gemischt hinterlassen; der Updater setzt sie weder zurück noch schließt er sie ab."),
    ("update_rollback_failed", "Eine unterbrochene frühere Aktualisierung ließ sich nicht zurücksetzen; die Meldung nennt den Schritt."),
    ("update_owner_still_alive", "Der Updater einer unterbrochenen Aktualisierung lief nach 30 Sekunden noch; dieser Lauf hat nichts geändert."),
    ("update_in_progress", "Ein anderer Update-Lauf hielt die Installation; dieser Lauf hat nichts geändert."),
    ("recovery_candidate_launcher_mismatch", "Die schon eingespielte Version gehört zu einem anderen Release als dieses Update (anderer MCP-Starter)."),
    ("runtime_sha256_mismatch", "Eine Programmdatei hatte nicht die SHA-256-Prüfsumme, die dieser Schritt erwartete. Updater bis einschließlich v0.5.198 meldeten so auch eine unterbrochene frühere Aktualisierung, die sie nicht zurücksetzen konnten: sie prüften dann die unveränderten Programmdateien gegen die neue Version."),
    ("update_deferred_busy", "Laufende Arbeit (ein MCP-Werkzeugaufruf oder ein Hintergrundauftrag) endete nicht innerhalb von 30 Sekunden; das Update hat nichts geändert."),
    ("lock_busy:", "Ein anderer Smart-Explorer-Prozess (MCP-Server, Hintergrunddienst oder ein zweiter Update-Lauf) hielt eine Sperre der Installation länger als 5 Sekunden."),
    ("update_recovery_required", "Eine frühere Aktualisierung war unterbrochen und sperrte die Installation."),
    ("laeuft noch und blockiert das Update", "Smart Explorer oder sein Hintergrunddienst lief nach 30 Sekunden noch; die Programmdateien wurden nicht ersetzt."),
    ("Administratorrechte", "Das Ersetzen braucht Administratorrechte; der Updater startet die App absichtlich nicht mit erhöhten Rechten."),
    ("Neue Version konnte nicht gestartet werden", "Die neue Version startete nicht oder meldete ihren Start nicht; die vorherige Version wurde wiederhergestellt."),
    ("Pruefsumme passt nicht", "Eine Datei hatte nicht die SHA-256-Prüfsumme der Update-Quelle; sie wurde nicht installiert."),
    ("Elternprozess", "Das Programm, das das Update startete, beendete sich nicht; das Update wurde nicht angewendet."),
];

/// The failure to show, or `None` when there is neither a report nor an
/// unfinished transaction (an update that merely runs is no failure).
pub fn explain(
    error: Option<&str>,
    pending: Option<PendingUpdate>,
    startup_check: bool,
) -> Option<UpdateFailure> {
    let error = error.map(str::trim).filter(|error| !error.is_empty());
    let pending = match (error, pending) {
        (None, None | Some(PendingUpdate::Running { .. })) => return None,
        (_, pending) => pending,
    };
    Some(UpdateFailure {
        reason: match error {
            Some(error) => reason(error).to_string(),
            None => unfinished_reason(pending.as_ref()),
        },
        next_action: next_action(pending.as_ref(), startup_check),
        error: error.map(str::to_string),
        pending,
    })
}

/// The reason for one report of the helper.
pub fn reason(error: &str) -> &'static str {
    known_reason(error).unwrap_or(
        "Das Update ist fehlgeschlagen; die Meldung des Updaters steht unverändert dabei.",
    )
}

/// The reason when the report carries one of the known codes.
pub fn known_reason(error: &str) -> Option<&'static str> {
    REASONS
        .iter()
        .find(|(marker, _)| error.contains(marker))
        .map(|(_, reason)| *reason)
}

fn unfinished_reason(pending: Option<&PendingUpdate>) -> String {
    match pending {
        Some(PendingUpdate::Unreadable { .. }) => {
            "Der Update-Zustand der Installation ist nicht lesbar.".to_string()
        }
        Some(
            PendingUpdate::RollBack { target_version, .. }
            | PendingUpdate::Complete { target_version, .. }
            | PendingUpdate::Refused { target_version, .. },
        ) => format!(
            "Die Aktualisierung auf v{target_version} wurde unterbrochen und sperrt die Installation."
        ),
        Some(PendingUpdate::Running { .. }) | None => String::new(),
    }
}

/// What happens next: an unfinished transaction is resolved by the next
/// update run, otherwise the next check offers the update again.
pub fn next_action(pending: Option<&PendingUpdate>, startup_check: bool) -> String {
    const BLOCKED: &str = "Bis dahin starten die MCP-Server dieser Installation nicht.";
    const RUN: &str = "`se update` oder „Installieren“ in der App";
    match pending {
        Some(PendingUpdate::RollBack {
            previous_version,
            target_version,
        }) => format!(
            "Der nächste Update-Lauf ({RUN}) setzt die unterbrochene Aktualisierung auf \
             v{previous_version} zurück und installiert danach v{target_version}. {BLOCKED}"
        ),
        Some(PendingUpdate::Complete { target_version, .. }) => format!(
            "Der nächste Update-Lauf ({RUN}) startet die schon eingespielte v{target_version} \
             und schließt die Aktualisierung ab. {BLOCKED}"
        ),
        Some(PendingUpdate::Refused {
            previous_version,
            target_version,
            detail,
        }) => format!(
            "Kein automatischer Weg: Update-Läufe und der Installer brechen ab, bis die \
             Programmdateien wieder alle v{previous_version} oder alle v{target_version} sind \
             ({detail}). {BLOCKED}"
        ),
        Some(PendingUpdate::Running { target_version }) => format!(
            "Ein Update-Lauf auf v{target_version} läuft gerade; danach steht hier sein Ergebnis."
        ),
        Some(PendingUpdate::Unreadable { detail }) => format!(
            "Der nächste Update-Lauf liest den Zustand erneut und bricht mit derselben Meldung \
             ab, solange er nicht lesbar ist ({detail})."
        ),
        None if startup_check => format!(
            "Die App sucht beim nächsten Start wieder nach dem Update und bietet es an; {RUN} \
             versucht es sofort."
        ),
        None => format!(
            "Kein automatischer neuer Versuch, weil die Suche beim Start aus ist; {RUN} oder \
             „Jetzt nach Updates suchen“ versucht es erneut."
        ),
    }
}
