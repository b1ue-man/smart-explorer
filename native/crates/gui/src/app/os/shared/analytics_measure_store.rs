//! Die Wahl „Sortieren/Kacheln nach“ der Speicheranalyse in
//! `<app data>/analytics_measure.txt` (`allocated` oder `size`). Eine fehlende
//! oder unlesbare Datei bedeutet den Standard „Belegt“, ohne Meldung; jede
//! Änderung durch die Person wird atomar geschrieben.
use super::analytics_measure::SizeMeasure;

const FILE: &str = "analytics_measure.txt";

pub(in crate::app) fn load() -> SizeMeasure {
    crate::support_dirs::read_private_text(&crate::support_dirs::app_data_file(FILE), 64)
        .ok()
        .and_then(|text| SizeMeasure::parse(text.trim()))
        .unwrap_or_default()
}

pub(in crate::app) fn save(measure: SizeMeasure) -> std::io::Result<()> {
    crate::support_dirs::write_private_atomic(
        &crate::support_dirs::app_data_file(FILE),
        measure.key().as_bytes(),
    )
}
