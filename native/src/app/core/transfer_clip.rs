//! The explorer's own clipboard entry. Ctrl+C only remembers what was
//! selected (side, entries, filter or the view's file snapshot); nothing is
//! scanned or downloaded until Ctrl+V starts the transfer. Whether the entry
//! is still what the clipboard holds is decided by the OS clipboard's sequence
//! numbers where the platform has a file clipboard (Windows), and by the text
//! the copy placed on the system clipboard elsewhere (Linux).
use super::transfer_route::TransferSelection;

pub(in crate::app) struct AppClip {
    pub(in crate::app) selection: TransferSelection,
    /// Ctrl+X: pasting moves (local folders only).
    pub(in crate::app) cut: bool,
    guard: ClipGuard,
    /// Marker text still to be written to the system clipboard.
    marker_pending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ClipGuard {
    /// OS clipboard sequence numbers under which this entry is current: the
    /// one before the copy (in case publishing for other programs failed or
    /// is still being prepared) and every one our own publications produced.
    Sequences(Vec<u32>),
    /// Text this copy placed on the system clipboard; a paste that brings
    /// other text means another program copied since.
    Marker(String),
    /// Nothing to compare with: the entry stays current until replaced.
    Unguarded,
}

impl AppClip {
    pub(in crate::app) fn new(selection: TransferSelection, cut: bool) -> Self {
        Self {
            selection,
            cut,
            guard: ClipGuard::Unguarded,
            marker_pending: false,
        }
    }

    /// Current while the OS clipboard shows `sequence` (or a later one of our
    /// own publications, see `add_sequence`).
    pub(in crate::app) fn guarded_by_sequence(mut self, sequence: Option<u32>) -> Self {
        if let Some(sequence) = sequence {
            self.guard = ClipGuard::Sequences(vec![sequence]);
        }
        self
    }

    /// Current while a paste brings exactly `marker` (written to the system
    /// clipboard at the end of the frame, see `take_pending_marker`).
    pub(in crate::app) fn guarded_by_marker(mut self, marker: String) -> Self {
        if !marker.trim().is_empty() {
            self.guard = ClipGuard::Marker(marker);
            self.marker_pending = true;
        }
        self
    }

    /// Our own publication (CF_HDROP, virtual files) changed the sequence.
    pub(in crate::app) fn add_sequence(&mut self, sequence: u32) {
        match &mut self.guard {
            ClipGuard::Sequences(accepted) => {
                if !accepted.contains(&sequence) {
                    accepted.push(sequence);
                }
            }
            guard => *guard = ClipGuard::Sequences(vec![sequence]),
        }
    }

    /// The marker text once, for the frame that can write the system clipboard.
    pub(in crate::app) fn take_pending_marker(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.marker_pending) {
            return None;
        }
        match &self.guard {
            ClipGuard::Marker(marker) => Some(marker.clone()),
            _ => None,
        }
    }

    /// Whether this entry is still what the clipboard holds. `sequence` is the
    /// OS clipboard's current sequence number (None when unreadable: the
    /// entry then wins); `pasted_text` is the text a keyboard paste brought
    /// (None for menu commands, which carry no clipboard text).
    pub(in crate::app) fn is_current(
        &self,
        sequence: Option<u32>,
        pasted_text: Option<&str>,
    ) -> bool {
        match &self.guard {
            ClipGuard::Sequences(accepted) => sequence.is_none_or(|now| accepted.contains(&now)),
            ClipGuard::Marker(marker) => {
                pasted_text.is_none_or(|text| normalized(text) == normalized(marker))
            }
            ClipGuard::Unguarded => true,
        }
    }

    pub(in crate::app) fn count(&self) -> usize {
        self.selection.count()
    }

    /// Feedback right after Ctrl+C / Ctrl+X.
    pub(in crate::app) fn copied_notice(&self) -> String {
        let n = self.count();
        if self.cut {
            format!("✂ {n} Element(e) ausgeschnitten – Strg+V verschiebt sie")
        } else {
            format!("✓ {n} Element(e) kopiert – Strg+V startet die Übertragung")
        }
    }
}

/// The clipboard text a copy leaves for other programs and as its marker:
/// one path per line.
pub(in crate::app) fn clip_marker_text(paths: &[String]) -> String {
    paths.join("\n")
}

/// Text as it comes back from a paste: the windowing layer turns CRLF into LF
/// and some clipboards add a final newline.
fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string()
}

#[cfg(test)]
#[path = "transfer_clip_tests.rs"]
mod tests;
