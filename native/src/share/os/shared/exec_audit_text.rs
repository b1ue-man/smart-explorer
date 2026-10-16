//! Text the other side supplies, as it appears in an audit line.
//!
//! Device names, claimed device ids, program names, working directories and
//! refusal reasons come from a peer that may be hostile or unauthenticated.
//! Each field is cut to `MAX_FIELD_CHARS` characters and loses every control
//! and direction-changing character: JSON escapes the ASCII controls, but a
//! terminal that shows `se share exec log` would still act on a raw C1
//! control such as U+009B, and U+202E would reorder the line on screen.

/// Longest peer-supplied field in characters. A device name is a label of a
/// few words, an EndpointId is 64 characters and the longest legitimate path
/// of a program or working directory fits well below; the digest of the whole
/// start (fixed length) still identifies a cut one. At most 4 bytes per
/// character and a dozen fields keep every line below 16 KiB, far under the
/// 4 MiB at which a file rotates.
pub(super) const MAX_FIELD_CHARS: usize = 256;

pub(super) fn limited(text: &str) -> String {
    let mut shown = String::with_capacity(text.len().min(MAX_FIELD_CHARS * 4));
    for (index, character) in text.chars().enumerate() {
        if index == MAX_FIELD_CHARS {
            shown.push('…');
            break;
        }
        shown.push(if misleading(character) {
            '?'
        } else {
            character
        });
    }
    shown
}

fn misleading(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}
