//! Protected omissions: other apps' private storage that Android hides from
//! every app since Android 11 (`<volume>/Android/data`, `<volume>/Android/obb`),
//! also with all-files access. A walk that meets such an area counts what it
//! could not read there instead of reporting read errors, so the result stays
//! complete when nothing else failed.
use std::sync::Mutex;

/// Areas kept per walk; a walk meets at most two per storage volume.
const MAX_PROTECTED_AREAS: usize = 16;

/// One protected area a walk passed through and how many of its entries it
/// could not read there (0 when Android hid the other apps' folders entirely).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedOmission {
    pub area: String,
    pub entries: u64,
}

#[derive(Default)]
pub(crate) struct ProtectedTally {
    areas: Mutex<Vec<ProtectedOmission>>,
}

impl ProtectedTally {
    /// The walk entered `area` (the folder itself or a root inside it).
    pub(crate) fn visit(&self, area: &str) {
        self.add(area, 0);
    }

    /// One entry inside `area` could not be read.
    pub(crate) fn omit(&self, area: &str) {
        self.add(area, 1);
    }

    fn add(&self, area: &str, entries: u64) {
        let mut areas = self.areas.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(known) = areas.iter_mut().find(|known| known.area == area) {
            known.entries = known.entries.saturating_add(entries);
        } else if areas.len() < MAX_PROTECTED_AREAS {
            areas.push(ProtectedOmission {
                area: area.to_string(),
                entries,
            });
        }
    }

    pub(crate) fn finish(self) -> Vec<ProtectedOmission> {
        let mut areas = self.areas.into_inner().unwrap_or_else(|p| p.into_inner());
        areas.sort_by(|left, right| left.area.cmp(&right.area));
        areas
    }
}

/// Entries that could not be read inside protected areas.
pub fn protected_count(omissions: &[ProtectedOmission]) -> u64 {
    omissions
        .iter()
        .fold(0u64, |total, area| total.saturating_add(area.entries))
}

/// The explanation for the analysis and duplicate pages; empty when the walk
/// met no protected area.
pub fn protected_text(omissions: &[ProtectedOmission]) -> String {
    if omissions.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "Android sperrt seit Android 11 die Ordner anderer Apps in „Android/data“ und \
         „Android/obb“ für jede App – auch mit „Zugriff auf alle Dateien“. Das ist eine \
         Grenze von Android, kein fehlendes Recht; diese Bereiche zählen nicht als Lesefehler."
            .to_string(),
    ];
    lines.extend(omissions.iter().map(area_line));
    lines.join("\n")
}

/// One line for the informational notes of a Share-hosted analysis, whose
/// wire format has no own field for protected omissions.
pub fn protected_note(omissions: &[ProtectedOmission]) -> Option<String> {
    if omissions.is_empty() {
        return None;
    }
    let areas: Vec<String> = omissions.iter().map(area_line).collect();
    Some(format!(
        "Geschützte App-Ordner (Android-Grenze, keine Lesefehler): {}",
        areas.join("; ")
    ))
}

fn area_line(area: &ProtectedOmission) -> String {
    match area.entries {
        0 => format!(
            "{}: Ordner anderer Apps von Android ausgeblendet",
            area.area
        ),
        1 => format!("{}: 1 Eintrag nicht lesbar", area.area),
        entries => format!("{}: {entries} Einträge nicht lesbar", area.area),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_background_task_protected_tally_merges_areas_and_texts() {
        let tally = ProtectedTally::default();
        tally.visit("/v/Android/obb");
        tally.omit("/v/Android/data");
        tally.omit("/v/Android/data");
        let omissions = tally.finish();
        assert_eq!(
            omissions,
            [
                ProtectedOmission {
                    area: "/v/Android/data".into(),
                    entries: 2,
                },
                ProtectedOmission {
                    area: "/v/Android/obb".into(),
                    entries: 0,
                },
            ]
        );
        assert_eq!(protected_count(&omissions), 2);
        let text = protected_text(&omissions);
        assert!(
            text.contains("/v/Android/data: 2 Einträge nicht lesbar"),
            "{text}"
        );
        assert!(
            text.contains("/v/Android/obb: Ordner anderer Apps"),
            "{text}"
        );
        let note = protected_note(&omissions).expect("note");
        assert!(note.starts_with("Geschützte App-Ordner"), "{note}");
        assert!(!note.contains('\n'));
        assert_eq!(protected_text(&[]), "");
        assert_eq!(protected_note(&[]), None);
    }
}
