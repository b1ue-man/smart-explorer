//! Models of creating, entering and managing vault sites in the desktop
//! windows (U29-5 to U29-7): the checks of the creation wizard, the text
//! under the location field, the draft of the lock settings and the
//! password change. No `egui`, no input/output.
#![allow(dead_code)] // some accessors are the acceptance signals (tests) and the surfaces of later packages
use super::vault_models::RecordModel;

/// Smallest and largest value of „Dateinamen kürzen ab“ (Cryptomator, with
/// the reason in `cryptomator::vault_files`).
pub(crate) const THRESHOLD_MIN: u32 = crate::cryptomator::MIN_SHORTENING_THRESHOLD;
pub(crate) const THRESHOLD_MAX: u32 = crate::cryptomator::MAX_SHORTENING_THRESHOLD;
pub(crate) const THRESHOLD_DEFAULT: u32 = crate::cryptomator::DEFAULT_SHORTENING_THRESHOLD;

/// Why a vault cannot be created at a location that already holds one.
pub(crate) const LOCATION_TAKEN: &str =
    "An diesem Ort gibt es schon einen Tresor. Mit „Tresor hinzufügen…“ eintragen.";
/// The mismatch of the two password fields.
pub(crate) const PASSWORDS_DIFFER: &str = "Die Passwörter stimmen nicht überein.";

/// „Dateinamen kürzen ab“: a whole number from 36 to 220.
pub(crate) fn parse_threshold(text: &str) -> Result<u32, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(THRESHOLD_DEFAULT);
    }
    match trimmed.parse::<u32>() {
        Ok(value) if (THRESHOLD_MIN..=THRESHOLD_MAX).contains(&value) => Ok(value),
        _ => Err(format!(
            "„Dateinamen kürzen ab“ braucht eine ganze Zahl von {THRESHOLD_MIN} bis {THRESHOLD_MAX}."
        )),
    }
}

/// Step 2 of the wizard: the new password and its repetition. The length
/// check of `vault::check_new_password` comes first (its text is shown
/// as it is), then the comparison.
pub(crate) fn password_step(password: &str, repeat: &str) -> Result<(), String> {
    crate::vault::check_new_password(password)?;
    if password != repeat {
        return Err(PASSWORDS_DIFFER.to_string());
    }
    Ok(())
}

/// What the location check found, as far as the windows need it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LocationView {
    pub(crate) is_vault: bool,
    pub(crate) format: Option<i64>,
    /// `masterkeyfile`, `hub` or `other`.
    pub(crate) key_source: Option<String>,
    /// Why se cannot open the vault found (Hub, format).
    pub(crate) problem: Option<String>,
}

/// The view of a `Detection` of the vault layer.
pub(crate) fn location_view(detection: &crate::vault::Detection) -> LocationView {
    LocationView {
        is_vault: detection.is_vault,
        format: detection.format,
        key_source: detection.key_source.map(str::to_string),
        problem: detection.problem.clone(),
    }
}

/// Step 1 of the wizard: a location that holds a vault is refused.
pub(crate) fn create_location_issue(found: &LocationView) -> Option<String> {
    found.is_vault.then(|| LOCATION_TAKEN.to_string())
}

/// What „Tresor hinzufügen…“ shows for a location and whether „Eintragen“
/// is possible.
pub(crate) fn add_summary(found: &LocationView) -> (String, bool) {
    if !found.is_vault {
        let text = found
            .problem
            .clone()
            .unwrap_or_else(|| "Kein Tresor an diesem Ort".to_string());
        return (text, false);
    }
    let format = found
        .format
        .map_or_else(|| "unbekannt".to_string(), |format| format.to_string());
    let key = match found.key_source.as_deref() {
        Some("masterkeyfile") => "Schlüsseldatei",
        Some("hub") => "Cryptomator Hub",
        _ => "anderer Schlüsselspeicher",
    };
    match &found.problem {
        Some(problem) => (format!("Tresor Format {format}, {key}: {problem}"), false),
        None => (format!("Tresor Format {format}, {key}"), true),
    }
}

/// The last name of a location as the default label of a new vault.
pub(crate) fn default_label(location: &str) -> String {
    location
        .trim()
        .split(['/', '\\'])
        .rev()
        .find(|part| !part.is_empty() && !part.ends_with(':'))
        .unwrap_or("Tresor")
        .to_string()
}

/// A label the vault list accepts.
pub(crate) fn label_issue(label: &str) -> Option<String> {
    if label.trim().is_empty() {
        return Some("Die Bezeichnung darf nicht leer sein.".to_string());
    }
    label
        .contains('/')
        .then(|| "Die Bezeichnung darf keinen / enthalten.".to_string())
}

/// The draft of the page „Sperren“ and the label of „Allgemein“.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsDraft {
    pub(crate) label: String,
    pub(crate) idle_on: bool,
    /// Minutes as typed.
    pub(crate) idle_text: String,
    pub(crate) lock_on_sleep: bool,
    pub(crate) lock_on_screen_lock: bool,
}

impl SettingsDraft {
    pub(crate) fn from_record(record: &RecordModel) -> Self {
        Self {
            label: record.display_label().to_string(),
            idle_on: record.auto_lock_idle_min.is_some(),
            idle_text: record
                .auto_lock_idle_min
                .unwrap_or(crate::vault::DEFAULT_IDLE_MINUTES)
                .to_string(),
            lock_on_sleep: record.lock_on_sleep,
            lock_on_screen_lock: record.lock_on_screen_lock,
        }
    }
}

/// The change from `record` to `draft`: only what differs (the vault layer
/// keeps the rest), or the text of the first wrong input.
pub(crate) fn settings_change(
    draft: &SettingsDraft,
    record: &RecordModel,
) -> Result<crate::vault::SettingsChange, String> {
    let mut change = crate::vault::SettingsChange::default();
    if let Some(issue) = label_issue(&draft.label) {
        return Err(issue);
    }
    if draft.label.trim() != record.display_label() {
        change.label = Some(draft.label.trim().to_string());
    }
    let idle = if draft.idle_on {
        match draft.idle_text.trim().parse::<u32>() {
            Ok(minutes) if minutes >= 1 => Some(minutes),
            _ => return Err("„Nach Leerlauf sperren“ braucht ganze Minuten ab 1.".to_string()),
        }
    } else {
        None
    };
    if idle != record.auto_lock_idle_min {
        change.auto_lock_idle_min = Some(idle);
    }
    if draft.lock_on_sleep != record.lock_on_sleep {
        change.lock_on_sleep = Some(draft.lock_on_sleep);
    }
    if draft.lock_on_screen_lock != record.lock_on_screen_lock {
        change.lock_on_screen_lock = Some(draft.lock_on_screen_lock);
    }
    Ok(change)
}

/// Whether a change has anything to save.
pub(crate) fn is_empty_change(change: &crate::vault::SettingsChange) -> bool {
    *change == crate::vault::SettingsChange::default()
}

/// The password page: old password, new password and its repetition.
pub(crate) fn password_change_step(old: &str, new: &str, repeat: &str) -> Result<(), String> {
    if old.is_empty() {
        return Err("Bitte das alte Passwort eingeben.".to_string());
    }
    password_step(new, repeat)
}

/// What the password page says after a change.
pub(crate) fn password_done_text(backup: &str, remember_error: Option<&str>) -> String {
    let mut text = "Passwort geändert.".to_string();
    if !backup.is_empty() {
        text.push_str(" Eine Sicherungsdatei wurde angelegt.");
    }
    if let Some(problem) = remember_error {
        text.push_str(&format!(
            " Das neue Passwort wurde nicht gemerkt: {problem}"
        ));
    }
    text
}

/// The hint line for a folder that holds a vault (`vault.cryptomator`, T2).
pub(crate) fn is_vault_file_name(name: &str) -> bool {
    name == "vault.cryptomator"
}
