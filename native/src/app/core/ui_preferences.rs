//! Portable presentation preferences. Storage belongs to the OS adapter.
use eframe::egui;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ColorMode {
    Light,
    Dark,
    #[default]
    System,
}

impl ColorMode {
    pub(super) fn preference(self) -> egui::ThemePreference {
        match self {
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
            Self::System => egui::ThemePreference::System,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Appearance {
    pub(super) mode: ColorMode,
    pub(super) compact: bool,
    pub(super) detailed_columns: bool,
}

impl Appearance {
    pub(super) fn row_height(self) -> f32 {
        if self.compact { 24.0 } else { 30.0 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct UiState {
    pub(in crate::app) show_filters: bool,
    pub(in crate::app) show_summary: bool,
    pub(super) appearance: Appearance,
}

impl UiState {
    pub(super) fn parse(text: &str) -> Self {
        let mut state = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let on = value == "1" || value.eq_ignore_ascii_case("true");
            match key.trim() {
                "show_filters" => state.show_filters = on,
                "show_summary" => state.show_summary = on,
                "color_mode" => state.appearance.mode = match value {
                    "light" => ColorMode::Light,
                    "dark" => ColorMode::Dark,
                    _ => ColorMode::System,
                },
                "compact" => state.appearance.compact = on,
                "detailed_columns" => state.appearance.detailed_columns = on,
                _ => {}
            }
        }
        state
    }

    pub(super) fn encode(self) -> String {
        format!(
            "show_filters={}\nshow_summary={}\ncolor_mode={}\ncompact={}\ndetailed_columns={}\n",
            self.show_filters as u8,
            self.show_summary as u8,
            self.appearance.mode.key(),
            self.appearance.compact as u8,
            self.appearance.detailed_columns as u8,
        )
    }
}
