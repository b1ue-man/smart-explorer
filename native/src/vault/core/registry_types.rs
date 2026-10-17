//! One entry of the vault list (C2.1): a vault site is the location of a
//! vault's root folder on any se location (`location`, the inner endpoint
//! locator) plus its settings, stored as `<AD>/vaults/<site-id>.conf` in
//! the `key=value` style of the sync jobs.
//!
//! Forward compatible: missing keys take their defaults, unknown keys are
//! kept and written back unchanged (a newer version's settings survive an
//! older version's save), a malformed known key makes the entry broken.
use super::locator::{is_site_id, is_vault_locator, VaultLocator, VAULT_STORE_SCHEME};

/// File name suffix of a site entry.
pub(crate) const SITE_FILE_SUFFIX: &str = ".conf";

/// Whether the site is the user's own vault or a copy a vault sync job
/// keeps (C5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SiteKind {
    Original,
    Replica,
}

impl SiteKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Replica => "replica",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "original" => Some(Self::Original),
            "replica" => Some(Self::Replica),
            _ => None,
        }
    }
}

/// When the vault locks by itself (C3.3). `None` minutes mean off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LockSettings {
    /// Lock after this many idle minutes (default off; 30 when switched on).
    pub(crate) auto_lock_idle_min: Option<u32>,
    pub(crate) lock_on_sleep: bool,
    /// Desktop: screen lock; Android: screen off.
    pub(crate) lock_on_screen_lock: bool,
    /// Android: minutes in the background (`0` = at once).
    pub(crate) lock_on_background_min: Option<u32>,
}

impl LockSettings {
    /// Desktop defaults (C3.3): idle off, sleep on, screen lock off.
    pub(crate) fn desktop() -> Self {
        Self {
            auto_lock_idle_min: None,
            lock_on_sleep: true,
            lock_on_screen_lock: false,
            lock_on_background_min: None,
        }
    }

    /// Android defaults (C3.3): idle off, screen off on, background 1 min.
    pub(crate) fn android() -> Self {
        Self {
            auto_lock_idle_min: None,
            lock_on_sleep: false,
            lock_on_screen_lock: true,
            lock_on_background_min: Some(1),
        }
    }
}

/// One vault site of the vault list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SiteRecord {
    /// Site ID: 32 lowercase hex digits.
    pub(crate) id: String,
    pub(crate) label: String,
    /// The se locator of the vault root folder (local path, saved remote,
    /// `gdrive://`, `share://`, …): never `vault-store://`.
    pub(crate) location: String,
    /// The vault ID (`jti` of `vault.cryptomator`).
    pub(crate) vault_id: String,
    pub(crate) kind: SiteKind,
    /// The original site of a replica.
    pub(crate) replica_of: Option<String>,
    /// A replica that has not finished its first complete sync (C5.9).
    pub(crate) seeding: bool,
    pub(crate) lock: LockSettings,
    pub(crate) remember_password: bool,
    pub(crate) created_ms: i64,
    /// Keys of newer versions, kept in their order.
    pub(crate) extra: Vec<(String, String)>,
}

impl SiteRecord {
    pub(crate) fn new(
        id: &str,
        label: &str,
        location: &str,
        vault_id: &str,
        created_ms: i64,
        lock: LockSettings,
    ) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            location: location.to_string(),
            vault_id: vault_id.to_string(),
            kind: SiteKind::Original,
            replica_of: None,
            seeding: false,
            lock,
            remember_password: false,
            created_ms,
            extra: Vec::new(),
        }
    }

    /// The label, or the ID when the label is empty.
    pub(crate) fn display_label(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.id
        } else {
            &self.label
        }
    }

    /// Refuses entries no se version may write: a bad ID, an empty or
    /// multi-line value, a vault inside its own storage view.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !is_site_id(&self.id) {
            return Err("Tresorort-ID muss aus 32 Hexziffern bestehen".into());
        }
        if self.label.contains('/') {
            return Err("Bezeichnung darf keinen / enthalten".into());
        }
        let texts = [&self.label, &self.location, &self.vault_id];
        if texts.iter().any(|text| text.contains(['\0', '\n', '\r'])) {
            return Err("Tresorort enthält ein Steuerzeichen".into());
        }
        if self.location.trim().is_empty() {
            return Err("Tresorort ohne Speicherort".into());
        }
        let location = self.location.trim_start();
        if location
            .split_once("://")
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case(VAULT_STORE_SCHEME))
        {
            return Err("Ein Tresor kann nicht in vault-store:// liegen".into());
        }
        if is_vault_locator(location)
            && VaultLocator::parse(location).is_ok_and(|inner| inner.site == self.id)
        {
            return Err("Ein Tresor kann nicht in sich selbst liegen".into());
        }
        if let Some(original) = &self.replica_of {
            if !is_site_id(original) {
                return Err("replica_of muss eine Tresorort-ID sein".into());
            }
        }
        Ok(())
    }

    /// The `.conf` body.
    pub(crate) fn serialize(&self) -> String {
        let mut out = String::from("# Smart Explorer vault site\n");
        out.push_str(&format!("id={}\n", self.id));
        push_text(&mut out, "label", &self.label);
        push_text(&mut out, "location", &self.location);
        push_text(&mut out, "vault_id", &self.vault_id);
        out.push_str(&format!("kind={}\n", self.kind.as_str()));
        let replica_of = self.replica_of.as_deref().unwrap_or("");
        out.push_str(&format!("replica_of={replica_of}\n"));
        push_bool(&mut out, "seeding", self.seeding);
        push_minutes(&mut out, "auto_lock_idle_min", self.lock.auto_lock_idle_min);
        push_bool(&mut out, "lock_on_sleep", self.lock.lock_on_sleep);
        push_bool(
            &mut out,
            "lock_on_screen_lock",
            self.lock.lock_on_screen_lock,
        );
        push_minutes(
            &mut out,
            "lock_on_background_min",
            self.lock.lock_on_background_min,
        );
        push_bool(&mut out, "remember_password", self.remember_password);
        out.push_str(&format!("created_ms={}\n", self.created_ms));
        for (key, value) in &self.extra {
            out.push_str(&format!("{key}={value}\n"));
        }
        out
    }

    /// Parses a `.conf` body; `defaults` are the lock settings of this
    /// platform for keys the entry does not carry.
    pub(crate) fn parse(text: &str, defaults: LockSettings) -> Result<Self, String> {
        let mut record = Self::new("", "", "", "", 0, defaults);
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .map(|(key, value)| (key.trim(), value.trim()))
                .ok_or_else(|| format!("Zeile {} ist kein key=value", index + 1))?;
            record.apply(key, value)?;
        }
        record.validate()?;
        Ok(record)
    }

    fn apply(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "id" => self.id = value.to_string(),
            "label" => self.label = value.to_string(),
            "label_json" => self.label = parse_json(key, value)?,
            "location" => self.location = value.to_string(),
            "location_json" => self.location = parse_json(key, value)?,
            "vault_id" => self.vault_id = value.to_string(),
            "vault_id_json" => self.vault_id = parse_json(key, value)?,
            "kind" => {
                self.kind = SiteKind::parse(value).ok_or_else(|| format!("kind {value:?}"))?
            }
            "replica_of" => {
                self.replica_of = (!value.is_empty()).then(|| value.to_string());
            }
            "seeding" => self.seeding = parse_bool(key, value)?,
            "auto_lock_idle_min" => self.lock.auto_lock_idle_min = parse_minutes(key, value)?,
            "lock_on_sleep" => self.lock.lock_on_sleep = parse_bool(key, value)?,
            "lock_on_screen_lock" => self.lock.lock_on_screen_lock = parse_bool(key, value)?,
            "lock_on_background_min" => {
                self.lock.lock_on_background_min = parse_minutes(key, value)?
            }
            "remember_password" => self.remember_password = parse_bool(key, value)?,
            "created_ms" => {
                self.created_ms = value.parse().map_err(|_| format!("created_ms {value:?}"))?
            }
            _ => self.extra.push((key.to_string(), value.to_string())),
        }
        Ok(())
    }
}

/// Plain while the value survives the trimmed line format, else as a JSON
/// string under `<key>_json` (no lossy fallback, as for sync endpoints).
fn push_text(out: &mut String, key: &str, value: &str) {
    if value.trim() == value && !value.contains(['\n', '\r']) {
        out.push_str(&format!("{key}={value}\n"));
    } else {
        let encoded = serde_json::Value::String(value.to_string());
        out.push_str(&format!("{key}_json={encoded}\n"));
    }
}

fn push_bool(out: &mut String, key: &str, value: bool) {
    out.push_str(&format!("{key}={}\n", u8::from(value)));
}

fn push_minutes(out: &mut String, key: &str, value: Option<u32>) {
    match value {
        Some(minutes) => out.push_str(&format!("{key}={minutes}\n")),
        None => out.push_str(&format!("{key}=off\n")),
    }
}

fn parse_json(key: &str, value: &str) -> Result<String, String> {
    serde_json::from_str(value).map_err(|_| format!("{key} ist kein JSON-Text"))
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(format!("{key} {value:?}")),
    }
}

fn parse_minutes(key: &str, value: &str) -> Result<Option<u32>, String> {
    if value == "off" {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|_| format!("{key} {value:?}"))
}

/// A new random site ID.
pub(crate) fn new_site_id() -> Result<String, String> {
    let mut bytes = [0u8; SITE_ID_BYTES];
    getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
    Ok(data_encoding::HEXLOWER.encode(&bytes))
}

const SITE_ID_BYTES: usize = 16;

/// The comparison key of two vault locations (a vault is entered once,
/// C2.1): backslashes as slashes, no trailing slash, drive and UNC paths
/// without letter case (Windows compares them that way).
pub(crate) fn location_key(location: &str) -> String {
    let location = location.trim();
    let bytes = location.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let unc = location.starts_with("\\\\") || location.starts_with("//");
    let mut key = if drive || unc {
        location.replace('\\', "/").to_lowercase()
    } else {
        location.to_string()
    };
    while key.len() > 1 && key.ends_with('/') && !key.ends_with("://") {
        key.pop();
    }
    key
}

/// `<id>.conf`.
pub(crate) fn site_file_name(id: &str) -> String {
    format!("{id}{SITE_FILE_SUFFIX}")
}

/// The site ID of a `<id>.conf` file name.
pub(crate) fn site_id_of_file(name: &str) -> Option<&str> {
    name.strip_suffix(SITE_FILE_SUFFIX)
        .filter(|id| is_site_id(id))
}
