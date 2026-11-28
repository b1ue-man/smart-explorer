//! Data model of the mask lists: who acts, entries, tolerance levels,
//! summaries without values, lockdown state and errors.
use serde::{Deserialize, Serialize};

/// Placeholder label of an entry without an own label.
pub const DEFAULT_LABEL: &str = "Maske";
/// Longest label in characters.
pub const MAX_LABEL_CHARS: usize = 40;
/// Longest list name in characters.
pub const MAX_NAME_CHARS: usize = 200;

/// Who asks for a change. Weakening changes (unlock, disable, remove) need
/// `VerifiedUser`: the caller confirmed the user through the system dialog of
/// the elevation (UAC/polkit) before it passes this actor. Clear text of the
/// entries reaches only `Gui`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Actor {
    /// The AI agent through the MCP server.
    Agent,
    /// `se masks` in a terminal (may be the agent through a shell).
    Cli,
    /// The desktop GUI of the user.
    Gui,
    /// A user confirmed by the elevation dialog for this one change.
    VerifiedUser,
}

/// Stable id of a list in the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ListId(pub u64);

/// Stable id of an entry in the store (unique over all lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(pub u64);

/// How an entry finds its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// The value as written; without Groß/Klein-Unterscheidung by default,
    /// optionally case-sensitive and whole word only.
    Literal,
    /// A Rust `regex` pattern; every distinct match gets its own placeholder.
    Regex,
    /// Fuzzy: folding, separators inside the match, the two-budget rule.
    Fuzzy,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Literal => "literal",
            Self::Regex => "regex",
            Self::Fuzzy => "fuzzy",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "literal" | "wörtlich" | "woertlich" | "exact" => Some(Self::Literal),
            "regex" => Some(Self::Regex),
            "fuzzy" | "unscharf" => Some(Self::Fuzzy),
            _ => None,
        }
    }
}

/// Tolerance of a fuzzy entry (plan S2): `Off` folds but allows no error,
/// `Low` allows inner errors after Elasticsearch `AUTO:3,6`, `Medium` also
/// cuts ⌊0.3·L⌋ characters at the word ends, `High` ⌊0.35·L⌋ and one inner
/// error more. `Custom` sets both budgets: `trim` characters at the ends,
/// `inner_tenths` weighted inner cost in tenths (1 error = 10, an OCR pair 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "level")]
pub enum Tolerance {
    Off,
    Low,
    Medium,
    High,
    Custom { trim: u32, inner_tenths: u32 },
}

impl Tolerance {
    /// Characters the match may lack at the two word ends together.
    pub fn trim(self, pattern_len: usize) -> usize {
        let limit = pattern_len.saturating_sub(1);
        let trim = match self {
            Self::Off | Self::Low => 0,
            Self::Medium => pattern_len * 3 / 10,
            Self::High => pattern_len * 35 / 100,
            Self::Custom { trim, .. } => trim as usize,
        };
        trim.min(limit)
    }

    /// Weighted inner cost (tenths) allowed on a core of `core_len` folded
    /// characters.
    pub fn inner_budget(self, core_len: usize) -> u32 {
        match self {
            Self::Off => 0,
            Self::Low | Self::Medium => auto_budget(core_len),
            Self::High => auto_budget(core_len) + 10,
            Self::Custom { inner_tenths, .. } => inner_tenths,
        }
    }

    /// Text form for surfaces: `off`, `low`, `medium`, `high` or
    /// `<trim>/<inner>` (inner in errors with one decimal).
    pub fn label(self) -> String {
        match self {
            Self::Off => "off".into(),
            Self::Low => "low".into(),
            Self::Medium => "medium".into(),
            Self::High => "high".into(),
            Self::Custom { trim, inner_tenths } => {
                format!("{trim}/{}.{}", inner_tenths / 10, inner_tenths % 10)
            }
        }
    }

    /// Parses the forms of [`Tolerance::label`] and the German level names
    /// (`aus`, `gering`, `mittel`, `hoch`).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_lowercase();
        match text.as_str() {
            "off" | "aus" => return Some(Self::Off),
            "low" | "gering" => return Some(Self::Low),
            "medium" | "mittel" => return Some(Self::Medium),
            "high" | "hoch" => return Some(Self::High),
            _ => {}
        }
        let (trim, inner) = text.split_once('/')?;
        let trim = trim.trim().parse::<u32>().ok()?;
        let inner = inner.trim().replace(',', ".");
        let inner = inner
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)?;
        let inner_tenths = (inner * 10.0).round();
        if inner_tenths > u32::MAX as f64 {
            return None;
        }
        Some(Self::Custom {
            trim,
            inner_tenths: inner_tenths as u32,
        })
    }

    /// Whether `self` finds at least everything `other` finds on a pattern
    /// of `pattern_len` folded characters (both budgets at least as large).
    pub fn covers(self, other: Self, pattern_len: usize) -> bool {
        self.trim(pattern_len) >= other.trim(pattern_len)
            && (1..=pattern_len.max(1))
                .all(|core| self.inner_budget(core) >= other.inner_budget(core))
    }
}

/// Elasticsearch `AUTO:3,6` in tenths: length 0–2 exact, 3–5 one error,
/// from 6 two errors.
pub fn auto_budget(len: usize) -> u32 {
    match len {
        0..=2 => 0,
        3..=5 => 10,
        _ => 20,
    }
}

/// A new entry as the surfaces hand it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEntry {
    pub kind: EntryKind,
    pub value: String,
    /// Placeholder label (letters, digits, `_`); `None` = [`DEFAULT_LABEL`].
    pub label: Option<String>,
    /// Literal only: distinguish Groß/Klein (default off).
    pub case_sensitive: bool,
    /// Literal only: match whole words only (default off).
    pub whole_word: bool,
    /// Fuzzy only.
    pub tolerance: Tolerance,
}

impl NewEntry {
    pub fn literal(value: impl Into<String>) -> Self {
        Self::new(EntryKind::Literal, value, Tolerance::Off)
    }

    pub fn regex(pattern: impl Into<String>) -> Self {
        Self::new(EntryKind::Regex, pattern, Tolerance::Off)
    }

    pub fn fuzzy(value: impl Into<String>, tolerance: Tolerance) -> Self {
        Self::new(EntryKind::Fuzzy, value, tolerance)
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    fn new(kind: EntryKind, value: impl Into<String>, tolerance: Tolerance) -> Self {
        Self {
            kind,
            value: value.into(),
            label: None,
            case_sensitive: false,
            whole_word: false,
            tolerance,
        }
    }
}

/// Fields of an entry to change; `None` keeps the field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryUpdate {
    pub value: Option<String>,
    pub label: Option<String>,
    pub case_sensitive: Option<bool>,
    pub whole_word: Option<bool>,
    pub tolerance: Option<Tolerance>,
}

/// An entry without its value: what every actor may see.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EntrySummary {
    pub id: EntryId,
    pub kind: EntryKind,
    pub label: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub tolerance: Tolerance,
}

/// An entry with its clear text value, only for [`Actor::Gui`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryWithValue {
    pub entry: EntrySummary,
    pub value: String,
    /// Real values the list has assigned placeholders to, with the
    /// placeholder (`⟦Label-N⟧`, variants `⟦Label-N.V⟧`).
    pub assigned: Vec<(String, String)>,
}

/// A list without values. `scopes` are the protected paths as the user
/// entered them; an agent surface masks them with every active list like any
/// other output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListSummary {
    pub id: ListId,
    pub name: String,
    pub enabled: bool,
    pub locked: bool,
    pub scopes: Vec<String>,
    pub entries: Vec<EntrySummary>,
    /// Number of real values with an assigned placeholder.
    pub assigned_values: usize,
}

/// What the agent hook reads without the store key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockdownState {
    /// No enabled list has a protected path.
    Off,
    /// An enabled list has a protected path: agents work only through se.
    On,
    /// The store exists (or existed) but cannot be read: treat as `On`.
    Unknown,
}

impl LockdownState {
    /// Fail-closed reading: `Unknown` blocks like `On`.
    pub fn blocks(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Errors of the management and the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaskError {
    /// The actor may not make this change; the text names the reason.
    NotAllowed(String),
    /// The list is locked by the user.
    Locked(String),
    NoSuchList(ListId),
    NoSuchEntry(EntryId),
    NoSuchScope(String),
    DuplicateName(String),
    InvalidEntry(String),
    InvalidScope(String),
    InvalidName(String),
    /// The store exists but cannot be read (missing key, wrong key, damaged,
    /// rolled back). Callers fail closed.
    Unreadable(String),
    Io(String),
}

impl std::fmt::Display for MaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAllowed(why) => write!(f, "Maskenliste: nicht erlaubt: {why}"),
            Self::Locked(name) => write!(f, "Maskenliste „{name}“ ist gesperrt"),
            Self::NoSuchList(id) => write!(f, "Keine Maskenliste mit der Kennung {}", id.0),
            Self::NoSuchEntry(id) => write!(f, "Kein Maskeneintrag mit der Kennung {}", id.0),
            Self::NoSuchScope(scope) => write!(f, "Kein Schutzpfad „{scope}“ in der Liste"),
            Self::DuplicateName(name) => write!(f, "Eine Maskenliste „{name}“ gibt es schon"),
            Self::InvalidEntry(why) => write!(f, "Maskeneintrag ungültig: {why}"),
            Self::InvalidScope(why) => write!(f, "Schutzpfad ungültig: {why}"),
            Self::InvalidName(why) => write!(f, "Name ungültig: {why}"),
            Self::Unreadable(why) => write!(f, "Maskenspeicher nicht lesbar: {why}"),
            Self::Io(why) => write!(f, "Maskenspeicher: {why}"),
        }
    }
}

impl std::error::Error for MaskError {}

/// Errors of [`crate::masks::MaskSet::unmask`]: the input is refused as a
/// whole, nothing of it reaches a file or a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnmaskError {
    /// A placeholder no enabled list assigned.
    Unknown(String),
    /// A `⟦` that starts neither a placeholder nor the escape `⟦⟦`.
    Malformed { offset: usize },
    /// The placeholder stands for bytes that are not UTF-8 (only
    /// `unmask_bytes` can restore it).
    NotText(String),
    /// The store could not be reread to resolve a placeholder.
    Store(String),
}

impl std::fmt::Display for UnmaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(text) => write!(f, "unbekannter Platzhalter {text}"),
            Self::Malformed { offset } => write!(
                f,
                "„⟦“ an Byte {offset} beginnt keinen Platzhalter; ein echtes „⟦“ wird als „⟦⟦“ geschrieben"
            ),
            Self::NotText(text) => {
                write!(f, "Platzhalter {text} steht für Binärdaten, nicht für Text")
            }
            Self::Store(why) => write!(f, "Platzhalter zurückführen: {why}"),
        }
    }
}

impl std::error::Error for UnmaskError {}

/// The store was changed outside se; the guard restored it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tampered {
    pub what: String,
}

impl std::fmt::Display for Tampered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Maskenspeicher von außen verändert ({}); wiederhergestellt",
            self.what
        )
    }
}

impl std::error::Error for Tampered {}

pub(crate) fn validate_label(label: &str) -> Result<(), MaskError> {
    let count = label.chars().count();
    if count == 0 || count > MAX_LABEL_CHARS {
        return Err(MaskError::InvalidEntry(format!(
            "Bezeichnung braucht 1 bis {MAX_LABEL_CHARS} Zeichen"
        )));
    }
    if !label.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(MaskError::InvalidEntry(
            "Bezeichnung nur aus Buchstaben, Ziffern und _".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<String, MaskError> {
    let name = name.trim();
    let count = name.chars().count();
    if count == 0 || count > MAX_NAME_CHARS {
        return Err(MaskError::InvalidName(format!(
            "Listenname braucht 1 bis {MAX_NAME_CHARS} Zeichen"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(MaskError::InvalidName(
            "Steuerzeichen im Listennamen".into(),
        ));
    }
    Ok(name.to_string())
}
