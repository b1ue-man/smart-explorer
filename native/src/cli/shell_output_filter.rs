//! Keeps one request of the remote terminal from reaching the local one: the
//! win32-input-mode (`CSI ? 9001 h`, Microsoft's ConPTY keyboard spec #4999).
//! A Windows pseudoconsole asks its terminal for that mode when it starts. A
//! local Windows console that obeys reports every key to `se` as a
//! `CSI Vk;Sc;Uc;Kd;Cs;Rc _` sequence instead of the key's bytes; `se` would
//! then never see a `~` at the start of a line, and `~.`, `~?` and `~~` would
//! stop working. The pseudoconsole reads plain keys as well. Every other byte
//! passes unchanged and in order (plan S4); Cygwin's pseudo-console output
//! filter drops the same request.
//!
//! A request split across reads is recognised: the start of a sequence that
//! may still become such a request waits for the next read, at most
//! `MAX_PENDING` bytes of it; a longer one passes unchanged. What still waits
//! when the session ends is at most an unfinished control sequence and is not
//! written.

/// The private mode number of the win32-input-mode.
const WIN32_INPUT_MODE: &[u8] = b"9001";
/// Bytes of an unfinished `CSI ?` sequence that are held back at most.
const MAX_PENDING: usize = 64;
const ESC: u8 = 0x1b;

/// Filters the remote terminal's output before it is written locally.
#[derive(Default)]
pub(super) struct OutputFilter {
    /// The start of a sequence that may be a mode request (`ESC`, `ESC [`,
    /// `ESC [ ?` and its parameter bytes); empty outside of one.
    pending: Vec<u8>,
}

impl OutputFilter {
    /// The bytes to write for the next read of the remote output, in order.
    pub(super) fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pending.len() + bytes.len());
        for &byte in bytes {
            self.push(byte, &mut out);
        }
        out
    }

    fn push(&mut self, byte: u8, out: &mut Vec<u8>) {
        match (self.pending.len(), byte) {
            (0, ESC) => self.pending.push(byte),
            (0, _) => out.push(byte),
            (1, b'[') | (2, b'?') => self.pending.push(byte),
            (length, b'0'..=b'9' | b';') if (3..MAX_PENDING).contains(&length) => {
                self.pending.push(byte)
            }
            (length, b'h' | b'l') if length >= 3 => {
                let sequence = std::mem::take(&mut self.pending);
                write_mode_request(&sequence[3..], byte, out);
            }
            _ => {
                // Not a mode request (or too long to hold): what was held back
                // goes out unchanged, and this byte starts over (it may be ESC).
                out.append(&mut self.pending);
                self.push(byte, out);
            }
        }
    }
}

/// Writes `CSI ? <parameters> <final_byte>` without the win32-input-mode; a
/// request without it stays byte for byte, one with nothing else is dropped.
fn write_mode_request(parameters: &[u8], final_byte: u8, out: &mut Vec<u8>) {
    let all: Vec<&[u8]> = parameters.split(|&byte| byte == b';').collect();
    let kept: Vec<&[u8]> = all
        .iter()
        .copied()
        .filter(|parameter| !is_win32_input_mode(parameter))
        .collect();
    if kept.len() < all.len() && kept.iter().all(|parameter| parameter.is_empty()) {
        return;
    }
    out.extend_from_slice(b"\x1b[?");
    if kept.len() == all.len() {
        out.extend_from_slice(parameters);
    } else {
        for (index, parameter) in kept.iter().enumerate() {
            if index > 0 {
                out.push(b';');
            }
            out.extend_from_slice(parameter);
        }
    }
    out.push(final_byte);
}

/// True for the parameter 9001, also with leading zeros.
fn is_win32_input_mode(parameter: &[u8]) -> bool {
    let start = parameter
        .iter()
        .position(|&byte| byte != b'0')
        .unwrap_or(parameter.len());
    parameter[start..] == *WIN32_INPUT_MODE
}

#[cfg(test)]
mod tests {
    use super::{OutputFilter, MAX_PENDING};

    fn filtered(parts: &[&[u8]]) -> Vec<u8> {
        let mut filter = OutputFilter::default();
        parts.iter().flat_map(|part| filter.filter(part)).collect()
    }

    fn once(bytes: &[u8]) -> Vec<u8> {
        filtered(&[bytes])
    }

    #[test]
    fn terminal_task_the_win32_input_mode_request_never_reaches_the_local_terminal() {
        assert_eq!(once(b"\x1b[?9001h"), b"");
        assert_eq!(once(b"\x1b[?9001l"), b"");
        assert_eq!(once(b"a\x1b[?9001hb\x1b[?09001lc"), b"abc");
        // The start of a Windows pseudoconsole: win32-input-mode, then focus events.
        assert_eq!(once(b"\x1b[?9001h\x1b[?1004h"), b"\x1b[?1004h");
    }

    #[test]
    fn terminal_task_other_modes_of_a_request_with_9001_stay() {
        assert_eq!(once(b"\x1b[?1004;9001h"), b"\x1b[?1004h");
        assert_eq!(once(b"\x1b[?9001;1004;2004l"), b"\x1b[?1004;2004l");
        assert_eq!(once(b"\x1b[?9001;9001h"), b"");
        assert_eq!(once(b"\x1b[?;9001h"), b"");
    }

    #[test]
    fn terminal_task_a_request_split_anywhere_is_filtered_like_one_read() {
        let whole: &[u8] = b"x\x1b[?1004;9001h\x1b[?2004hy\x1b[31mz\x1b[?9001l";
        let expected = once(whole);
        assert_eq!(expected, b"x\x1b[?1004h\x1b[?2004hy\x1b[31mz");
        for split in 0..=whole.len() {
            let (first, second) = whole.split_at(split);
            assert_eq!(filtered(&[first, second]), expected, "split at {split}");
        }
        let bytes: Vec<&[u8]> = whole.chunks(1).collect();
        assert_eq!(filtered(&bytes), expected);
    }

    #[test]
    fn terminal_task_other_output_passes_unchanged() {
        let cases: [&[u8]; 6] = [
            b"plain text \xc3\xa9 and \r\n",
            b"\x1b[?2004h\x1b[?1049h\x1b[?25l\x1b[?1004l\x1b[?h",
            b"\x1b[31mred\x1b[0m\x1b[2J\x1b[H",
            b"\x1b]0;title\x07\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\",
            b"\x1b[?90011h\x1b[?900h\x1b[9001h\x1bx\x1b[?1;2x",
            b"\x1b\x1b[?2004h\x1b[\x1b[?1h",
        ];
        for case in cases {
            assert_eq!(once(case), case, "{case:?}");
        }
    }

    #[test]
    fn terminal_task_held_back_bytes_are_bounded() {
        // A request longer than the limit is not held back any further and
        // passes unchanged, 9001 included.
        let mut long = b"\x1b[?".to_vec();
        for _ in 0..40 {
            long.extend_from_slice(b"1;");
        }
        long.extend_from_slice(b"9001h");
        let mut filter = OutputFilter::default();
        let mut out = Vec::new();
        for byte in &long {
            out.extend(filter.filter(std::slice::from_ref(byte)));
            assert!(filter.pending.len() <= MAX_PENDING);
        }
        assert_eq!(out, long);
        assert!(filter.pending.is_empty());
    }
}
