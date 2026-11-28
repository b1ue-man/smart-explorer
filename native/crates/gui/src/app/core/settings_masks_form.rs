//! Entry form of the page „Maskenlisten“: kind, value (hidden while typing),
//! label, tolerance as level (aus, gering, mittel, hoch) or numbers (Kürzung
//! an den Wortenden, Fehler innen), Groß/Klein and ganzes Wort.
use super::super::super::prelude::*;
use crate::masks::{EntryKind, EntrySummary, EntryUpdate, NewEntry, Tolerance};

/// Level choice of the form; `Numbers` uses `trim` and `inner`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum Level {
    Off,
    Low,
    #[default]
    Medium,
    High,
    Numbers,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct EntryForm {
    pub(in crate::app) kind: EntryKind,
    pub(in crate::app) value: String,
    pub(in crate::app) label: String,
    pub(in crate::app) level: Level,
    pub(in crate::app) trim: u32,
    /// Inner errors (1 error = 1.0, an OCR pair 0.4).
    pub(in crate::app) inner: f32,
    pub(in crate::app) case_sensitive: bool,
    pub(in crate::app) whole_word: bool,
    pub(in crate::app) show_value: bool,
}

impl Default for EntryForm {
    fn default() -> Self {
        Self {
            kind: EntryKind::Literal,
            value: String::new(),
            label: String::new(),
            level: Level::Medium,
            trim: 2,
            inner: 1.0,
            case_sensitive: false,
            whole_word: false,
            show_value: false,
        }
    }
}

impl EntryForm {
    pub(in crate::app) fn tolerance(&self) -> Tolerance {
        match self.level {
            Level::Off => Tolerance::Off,
            Level::Low => Tolerance::Low,
            Level::Medium => Tolerance::Medium,
            Level::High => Tolerance::High,
            Level::Numbers => Tolerance::Custom {
                trim: self.trim,
                inner_tenths: (self.inner.max(0.0) * 10.0).round() as u32,
            },
        }
    }

    pub(in crate::app) fn from_summary(entry: &EntrySummary) -> Self {
        let mut form = Self {
            kind: entry.kind,
            label: entry.label.clone(),
            case_sensitive: entry.case_sensitive,
            whole_word: entry.whole_word,
            ..Self::default()
        };
        form.level = match entry.tolerance {
            Tolerance::Off => Level::Off,
            Tolerance::Low => Level::Low,
            Tolerance::Medium => Level::Medium,
            Tolerance::High => Level::High,
            Tolerance::Custom { trim, inner_tenths } => {
                form.trim = trim;
                form.inner = inner_tenths as f32 / 10.0;
                Level::Numbers
            }
        };
        form
    }

    /// The new entry, or why the form is incomplete.
    pub(in crate::app) fn entry(&self) -> Result<NewEntry, String> {
        if self.value.is_empty() {
            return Err("Wert fehlt".into());
        }
        let mut entry = match self.kind {
            EntryKind::Literal => NewEntry::literal(self.value.clone()),
            EntryKind::Regex => NewEntry::regex(self.value.clone()),
            EntryKind::Fuzzy => NewEntry::fuzzy(self.value.clone(), self.tolerance()),
        };
        let label = self.label.trim();
        entry.label = (!label.is_empty()).then(|| label.to_string());
        entry.case_sensitive = self.kind == EntryKind::Literal && self.case_sensitive;
        entry.whole_word = self.kind == EntryKind::Literal && self.whole_word;
        Ok(entry)
    }

    /// Only the fields that differ from `original`; an empty value keeps it.
    pub(in crate::app) fn update(&self, original: &EntrySummary) -> EntryUpdate {
        let label = self.label.trim().to_string();
        let tolerance = self.tolerance();
        EntryUpdate {
            value: (!self.value.is_empty()).then(|| self.value.clone()),
            label: (label != original.label && !label.is_empty()).then_some(label),
            case_sensitive: (self.case_sensitive != original.case_sensitive)
                .then_some(self.case_sensitive),
            whole_word: (self.whole_word != original.whole_word).then_some(self.whole_word),
            tolerance: (original.kind == EntryKind::Fuzzy && tolerance != original.tolerance)
                .then_some(tolerance),
        }
    }
}

/// One line describing an entry without its value.
pub(in crate::app) fn describe(entry: &EntrySummary) -> String {
    let kind = match entry.kind {
        EntryKind::Literal => "wörtlich",
        EntryKind::Regex => "Regex",
        EntryKind::Fuzzy => "unscharf",
    };
    let mut parts = vec![kind.to_string()];
    if entry.kind == EntryKind::Fuzzy {
        parts.push(format!("Toleranz {}", level_text(entry.tolerance)));
    }
    if entry.kind == EntryKind::Literal {
        parts.push(
            if entry.case_sensitive {
                "Groß/Klein"
            } else {
                "ohne Groß/Klein"
            }
            .into(),
        );
        if entry.whole_word {
            parts.push("ganzes Wort".into());
        }
    }
    parts.join(" · ")
}

pub(in crate::app) fn level_text(tolerance: Tolerance) -> String {
    match tolerance {
        Tolerance::Off => "aus".into(),
        Tolerance::Low => "gering".into(),
        Tolerance::Medium => "mittel".into(),
        Tolerance::High => "hoch".into(),
        Tolerance::Custom { trim, inner_tenths } => format!(
            "Kürzung {trim}, innen {},{}",
            inner_tenths / 10,
            inner_tenths % 10
        ),
    }
}

/// Draws the form; `true` when „Hinzufügen“/„Übernehmen“ was clicked. When
/// editing (`adding = false`) the value field may stay empty (keeps the
/// value).
pub(in crate::app) fn form_ui(
    ui: &mut egui::Ui,
    form: &mut EntryForm,
    salt: (&str, u64),
    adding: bool,
) -> bool {
    let mut submit = false;
    ui.horizontal_wrapped(|ui| {
        if adding {
            egui::ComboBox::from_id_salt((salt.0, salt.1, "kind"))
                .selected_text(match form.kind {
                    EntryKind::Literal => "wörtlich",
                    EntryKind::Regex => "Regex",
                    EntryKind::Fuzzy => "unscharf",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut form.kind, EntryKind::Literal, "wörtlich");
                    ui.selectable_value(&mut form.kind, EntryKind::Regex, "Regex");
                    ui.selectable_value(&mut form.kind, EntryKind::Fuzzy, "unscharf");
                });
        }
        ui.add(
            egui::TextEdit::singleline(&mut form.value)
                .password(!form.show_value)
                .hint_text(if adding {
                    "Wert"
                } else {
                    "neuer Wert (leer = bleibt)"
                })
                .desired_width(220.0),
        );
        ui.checkbox(&mut form.show_value, "sichtbar");
        ui.add(
            egui::TextEdit::singleline(&mut form.label)
                .hint_text("Bezeichnung (Maske)")
                .desired_width(120.0),
        );
    });
    ui.horizontal_wrapped(|ui| match form.kind {
        EntryKind::Literal => {
            ui.checkbox(&mut form.case_sensitive, "Groß/Klein unterscheiden");
            ui.checkbox(&mut form.whole_word, "Nur ganzes Wort");
        }
        EntryKind::Regex => {
            ui.label(RichText::new("Rust-Regex, mehrzeilig möglich (\\s, \\n, \\d).").small());
        }
        EntryKind::Fuzzy => {
            ui.label("Toleranz:");
            for (level, label) in [
                (Level::Off, "aus"),
                (Level::Low, "gering"),
                (Level::Medium, "mittel"),
                (Level::High, "hoch"),
                (Level::Numbers, "Zahlen"),
            ] {
                ui.radio_value(&mut form.level, level, label);
            }
            if form.level == Level::Numbers {
                ui.label("Kürzung");
                ui.add(egui::DragValue::new(&mut form.trim).range(0..=20));
                ui.label("Fehler innen");
                ui.add(
                    egui::DragValue::new(&mut form.inner)
                        .range(0.0..=10.0)
                        .speed(0.1),
                );
            }
        }
    });
    let ready = !adding || !form.value.is_empty();
    ui.horizontal_wrapped(|ui| {
        let label = if adding { "Hinzufügen" } else { "Übernehmen" };
        submit = ui.add_enabled(ready, egui::Button::new(label)).clicked();
    });
    submit
}
