//! „Größe“ und „Belegt“ der Speicheranalyse (2026-11-18). Ein Baumknoten
//! trägt beide Werte (`SizeNode::size`, `SizeNode::allocated`); die Liste
//! zeigt beide, Treemap-Kacheln und Sortierung folgen der Wahl „Sortieren/
//! Kacheln nach“. Standard ist „Belegt“; kennt das Ergebnis den belegten
//! Platz nicht (ältere Gegenstelle, Backend ohne Wert), gilt „Größe“ und die
//! Liste sagt „Belegt: unbekannt“.
use crate::analytics::SizeNode;
use crate::format::format_bytes;

/// Was Sortierung und Kachelfläche bestimmt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum SizeMeasure {
    /// Platz auf dem Datenträger.
    #[default]
    Allocated,
    /// Summe der Dateilängen.
    Size,
}

impl SizeMeasure {
    pub(in crate::app) const ALL: [Self; 2] = [Self::Allocated, Self::Size];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Allocated => "Belegt",
            Self::Size => "Größe",
        }
    }

    /// Gespeicherter Name (und Wert von `se analyze --sort`).
    pub(in crate::app) fn key(self) -> &'static str {
        match self {
            Self::Allocated => "allocated",
            Self::Size => "size",
        }
    }

    pub(in crate::app) fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|measure| measure.key() == text)
    }

    pub(in crate::app) fn of(self, node: &SizeNode) -> u64 {
        match self {
            Self::Allocated => node.allocated,
            Self::Size => node.size,
        }
    }
}

/// Tooltip des Umschalters und der Spaltenköpfe.
pub(in crate::app) const MEASURE_HINT: &str = "Größe: Summe der Dateilängen.\n\
Belegt: Platz, den die Dateien auf dem Datenträger tatsächlich einnehmen.\n\
Beides weicht ab bei Platzhaltern „nur online“ (Cloud-Dateien liegen nicht auf \
dem Datenträger), Kompression (komprimierte Dateien belegen weniger), Sparse-\
Dateien (Lücken belegen nichts) und Hardlinks (mehrfach verlinkte Dateien \
belegen ihren Platz nur einmal).\nBeim Löschen wird der belegte Platz frei.";

/// Text, wenn das Ergebnis den belegten Platz nicht kennt.
pub(in crate::app) const ALLOCATED_UNKNOWN: &str = "unbekannt";

/// Gespeicherte Wahl und was das angezeigte Ergebnis weiß.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct MeasureView {
    /// Wahl der Person (gespeichert); Standard „Belegt“.
    pub(in crate::app) chosen: SizeMeasure,
    /// Das Ergebnis kennt den belegten Platz (`ScanOutcome::allocated_known`).
    pub(in crate::app) allocated_known: bool,
}

impl MeasureView {
    /// Vor einem Ergebnis ist der belegte Platz unbekannt.
    pub(in crate::app) fn new(chosen: SizeMeasure) -> Self {
        Self {
            chosen,
            allocated_known: false,
        }
    }

    /// Was gilt: ohne bekannten belegten Platz immer „Größe“.
    pub(in crate::app) fn effective(self) -> SizeMeasure {
        if self.allocated_known {
            self.chosen
        } else {
            SizeMeasure::Size
        }
    }

    /// Fläche und Sortierschlüssel eines Knotens.
    pub(in crate::app) fn weight(self, node: &SizeNode) -> u64 {
        self.effective().of(node)
    }

    pub(in crate::app) fn allocated_text(self, allocated: u64) -> String {
        if self.allocated_known {
            format_bytes(allocated)
        } else {
            ALLOCATED_UNKNOWN.to_string()
        }
    }

    /// `Größe 5.00 GB · Belegt 100 MB` (Gesamtzeile, Tooltip, Vorlesetext).
    pub(in crate::app) fn both_text(self, size: u64, allocated: u64) -> String {
        format!(
            "Größe {} · Belegt {}",
            format_bytes(size),
            self.allocated_text(allocated)
        )
    }

    /// Ordnet die Kinder jedes Ordners einmal absteigend nach dem geltenden
    /// Maß, gleich große nach Namen. Treemap und Liste lesen die Kinder dann
    /// in dieser Reihenfolge, ohne je Bild zu sortieren.
    pub(in crate::app) fn sort_tree(self, tree: &mut SizeNode) {
        let measure = self.effective();
        let mut stack = vec![tree];
        while let Some(node) = stack.pop() {
            node.children.sort_by(|left, right| {
                measure
                    .of(right)
                    .cmp(&measure.of(left))
                    .then_with(|| left.name.cmp(&right.name))
            });
            stack.extend(node.children.iter_mut().filter(|child| child.is_dir));
        }
    }
}

/// Sortiert den Baum eines Ergebnisses nach `chosen` (im Scan-Thread).
pub(in crate::app) fn sort_outcome(
    outcome: &mut crate::analytics::ScanOutcome,
    chosen: SizeMeasure,
) {
    let view = MeasureView {
        chosen,
        allocated_known: outcome.allocated_known,
    };
    if let Some(tree) = outcome.tree.as_mut() {
        view.sort_tree(tree);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, size: u64, allocated: u64) -> SizeNode {
        SizeNode {
            name: name.into(),
            size,
            allocated,
            is_dir: true,
            children: Vec::new(),
        }
    }

    #[test]
    fn belegt_task_desktop_sorts_by_allocated_and_falls_back_to_size_when_unknown() {
        // „gelöscht“ zeigt 5 GB, belegt aber 100 MB (Platzhalter nur online).
        let children = vec![
            node("geloescht", 5 << 30, 100 << 20),
            node("fotos", 2 << 30, 2 << 30),
            node("leer", 0, 0),
        ];
        assert_eq!(SizeMeasure::default(), SizeMeasure::Allocated);
        let known = MeasureView {
            chosen: SizeMeasure::default(),
            allocated_known: true,
        };
        let names = |view: MeasureView| -> Vec<String> {
            let mut root = node("root", 0, 0);
            root.children = children
                .iter()
                .map(|c| node(&c.name, c.size, c.allocated))
                .collect();
            view.sort_tree(&mut root);
            root.children
                .iter()
                .map(|child| child.name.to_string())
                .collect()
        };
        assert_eq!(names(known), ["fotos", "geloescht", "leer"]);
        assert_eq!(known.weight(&children[0]), 100 << 20);
        let by_size = MeasureView {
            chosen: SizeMeasure::Size,
            ..known
        };
        assert_eq!(names(by_size), ["geloescht", "fotos", "leer"]);
        // Ohne bekannten belegten Platz: Standard „Größe“, Anzeige „unbekannt“.
        let unknown = MeasureView::new(SizeMeasure::Allocated);
        assert_eq!(unknown.effective(), SizeMeasure::Size);
        assert_eq!(names(unknown), ["geloescht", "fotos", "leer"]);
        assert_eq!(unknown.allocated_text(100 << 20), "unbekannt");
        assert_eq!(
            unknown.both_text(5 << 30, 5 << 30),
            format!("Größe {} · Belegt unbekannt", format_bytes(5 << 30))
        );
        assert_eq!(
            known.both_text(5 << 30, 100 << 20),
            format!(
                "Größe {} · Belegt {}",
                format_bytes(5 << 30),
                format_bytes(100 << 20)
            )
        );
        for measure in SizeMeasure::ALL {
            assert_eq!(SizeMeasure::parse(measure.key()), Some(measure));
        }
        assert_eq!(SizeMeasure::parse("blocks"), None);
        assert!(MEASURE_HINT.contains("nur online") && MEASURE_HINT.contains("Hardlinks"));
    }
}
