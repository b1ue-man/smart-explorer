//! „Größte Dateien“ und „Nach Dateityp“ einer Speicheranalyse (2026-11-18):
//! die größten Dateien des ganzen Baums und je Endung die Summen von Größe
//! und belegtem Platz. Eine reine Funktion über `SizeNode` im Speicher-Kern
//! (`crate::analytics::overview`), die der Desktop
//! (`gui/src/app/core/analytics_overview_ui.rs`, als `crate::analytics_overview`),
//! `se analyze --files --types` und die Android-Brücke (`analyze.overview`)
//! gemeinsam nutzen.
//!
//! Dateien, die ein Verzeichnis über das Analysebudget hinaus hatte, stehen
//! nur als Sammelknoten „… N weitere Eintraege“ im Baum: sie haben weder Namen
//! noch Endung. Sie zählen in keine Typensumme und keine Liste, sondern in
//! `aggregated` (Anzahl, Größe, belegt), damit die Ansicht sagt, wie viel
//! nicht nach Typ aufgeteilt werden konnte.
use crate::analytics::{is_aggregate_name, SizeNode};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// Endungen, die einzeln geführt werden; der Rest wird eine Zeile.
pub const MAX_TYPES: usize = 200;
/// Name der Zeile für Dateien ohne Endung.
pub const NO_EXTENSION: &str = "(ohne Endung)";
/// Name der Zeile, die die Endungen jenseits von [`MAX_TYPES`] sammelt.
pub const OTHER_TYPES: &str = "(weitere Endungen)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LargestFile {
    /// Pfad unter der Analysewurzel, mit `/` getrennt.
    pub path: String,
    pub size: u64,
    pub allocated: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeTotal {
    /// Endung in Kleinbuchstaben ohne Punkt, [`NO_EXTENSION`] oder [`OTHER_TYPES`].
    pub extension: String,
    pub files: u64,
    pub size: u64,
    pub allocated: u64,
}

/// Was in Sammelknoten steckt und daher keinem Typ zugeordnet ist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Aggregated {
    pub files: u64,
    pub size: u64,
    pub allocated: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overview {
    /// Absteigend nach der Messgröße, höchstens `top`.
    pub largest: Vec<LargestFile>,
    /// Absteigend nach der Messgröße.
    pub types: Vec<TypeTotal>,
    pub aggregated: Aggregated,
}

/// `by_allocated`: nach belegtem Platz statt nach Größe ordnen und auswählen.
pub fn overview(tree: &SizeNode, by_allocated: bool, top: usize) -> Overview {
    let weight = |size: u64, allocated: u64| if by_allocated { allocated } else { size };
    let mut types: HashMap<String, TypeTotal> = HashMap::new();
    let mut aggregated = Aggregated::default();
    // Min-heap of the `top` largest: (weight, Reverse(path)) keeps equal
    // weights in name order.
    let mut heap: BinaryHeap<Reverse<(u64, Reverse<String>, u64, u64)>> = BinaryHeap::new();
    let mut stack: Vec<(&SizeNode, String)> = vec![(tree, String::new())];
    while let Some((node, prefix)) = stack.pop() {
        for child in &node.children {
            if child.is_dir {
                stack.push((child, join(&prefix, &child.name)));
                continue;
            }
            if is_aggregate_name(&child.name) {
                aggregated.files = aggregated
                    .files
                    .saturating_add(aggregate_count(&child.name));
                aggregated.size = aggregated.size.saturating_add(child.size);
                aggregated.allocated = aggregated.allocated.saturating_add(child.allocated);
                continue;
            }
            let total = types
                .entry(extension(&child.name))
                .or_insert_with_key(|extension| TypeTotal {
                    extension: extension.clone(),
                    ..TypeTotal::default()
                });
            total.files += 1;
            total.size = total.size.saturating_add(child.size);
            total.allocated = total.allocated.saturating_add(child.allocated);
            if top == 0 {
                continue;
            }
            let key = weight(child.size, child.allocated);
            let enters = heap.len() < top || heap.peek().is_some_and(|Reverse(min)| key > min.0);
            if enters {
                let path = join(&prefix, &child.name);
                heap.push(Reverse((key, Reverse(path), child.size, child.allocated)));
                if heap.len() > top {
                    heap.pop();
                }
            }
        }
    }
    let mut largest: Vec<LargestFile> = heap
        .into_iter()
        .map(|Reverse((_, Reverse(path), size, allocated))| LargestFile {
            path,
            size,
            allocated,
        })
        .collect();
    largest.sort_by(|left, right| {
        weight(right.size, right.allocated)
            .cmp(&weight(left.size, left.allocated))
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut types: Vec<TypeTotal> = types.into_values().collect();
    types.sort_by(|left, right| {
        weight(right.size, right.allocated)
            .cmp(&weight(left.size, left.allocated))
            .then_with(|| left.extension.cmp(&right.extension))
    });
    if types.len() > MAX_TYPES {
        let folded = types.split_off(MAX_TYPES - 1);
        let mut other = TypeTotal {
            extension: OTHER_TYPES.to_string(),
            ..TypeTotal::default()
        };
        for total in folded {
            other.files = other.files.saturating_add(total.files);
            other.size = other.size.saturating_add(total.size);
            other.allocated = other.allocated.saturating_add(total.allocated);
        }
        types.push(other);
    }
    Overview {
        largest,
        types,
        aggregated,
    }
}

/// „X Dateien zusammengefasst …“ oder `None` ohne Sammelknoten.
pub fn aggregated_note(aggregated: &Aggregated) -> Option<String> {
    (aggregated.files > 0).then(|| {
        format!(
            "{} Dateien zusammengefasst (große Ordner): sie zählen in keinen Dateityp und in keine Liste.",
            aggregated.files
        )
    })
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            extension.to_lowercase()
        }
        _ => NO_EXTENSION.to_string(),
    }
}

/// `N` of „… N weitere Eintraege“ (the name is checked by `is_aggregate_name`).
fn aggregate_count(name: &str) -> u64 {
    name.split_whitespace()
        .find_map(|part| part.parse::<u64>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, allocated: u64) -> SizeNode {
        SizeNode {
            name: name.into(),
            size,
            allocated,
            is_dir: false,
            children: Vec::new(),
        }
    }

    fn dir(name: &str, children: Vec<SizeNode>) -> SizeNode {
        SizeNode {
            name: name.into(),
            size: children.iter().map(|child| child.size).sum(),
            allocated: children.iter().map(|child| child.allocated).sum(),
            is_dir: true,
            children,
        }
    }

    #[test]
    fn analyse_task_overview_lists_largest_files_and_types_apart_from_aggregates() {
        let aggregate = crate::analytics::aggregate_name(7);
        let tree = dir(
            "root",
            vec![
                dir(
                    "geloescht",
                    vec![file("film.MKV", 5000, 100), file(&aggregate, 700, 700)],
                ),
                file("foto.jpg", 2000, 2000),
                file("notiz.txt", 10, 4096),
                file(".bashrc", 1, 4096),
            ],
        );
        let by_allocated = overview(&tree, true, 2);
        let paths: Vec<&str> = by_allocated
            .largest
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(paths, [".bashrc", "notiz.txt"]);
        let by_size = overview(&tree, false, 2);
        let paths: Vec<&str> = by_size.largest.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["geloescht/film.MKV", "foto.jpg"]);
        assert_eq!(by_size.largest[0].allocated, 100);

        let mkv = by_size.types.iter().find(|t| t.extension == "mkv");
        assert_eq!(
            mkv.map(|t| (t.files, t.size, t.allocated)),
            Some((1, 5000, 100))
        );
        assert!(by_size.types.iter().any(|t| t.extension == NO_EXTENSION));
        assert_eq!(by_size.types.iter().map(|t| t.files).sum::<u64>(), 4);
        assert_eq!(
            by_size.aggregated,
            Aggregated {
                files: 7,
                size: 700,
                allocated: 700
            }
        );
        let note = aggregated_note(&by_size.aggregated).unwrap_or_default();
        assert!(note.starts_with("7 Dateien zusammengefasst"), "{note}");
        assert_eq!(aggregated_note(&Aggregated::default()), None);

        let many = dir(
            "root",
            (0..MAX_TYPES + 5)
                .map(|index| file(&format!("f.e{index}"), 1, 1))
                .collect(),
        );
        let folded = overview(&many, false, 0);
        assert!(folded.largest.is_empty());
        assert_eq!(folded.types.len(), MAX_TYPES);
        let other = folded.types.last().map(|t| (t.extension.as_str(), t.files));
        assert_eq!(other, Some((OTHER_TYPES, 6)));
    }
}
