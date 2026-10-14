//! The escape sequences of a `se shell` terminal session, as `ssh(1)` defines
//! them (section ESCAPE CHARACTERS): the escape character counts only at the
//! start of the session and directly after a typed carriage return or line
//! feed. Then `.` leaves, `?` shows the help, the escape character itself sends
//! one escape character, and any other byte sends the escape character and that
//! byte. A pending escape character survives the end of one read, so a sequence
//! split over two reads works. Everything else passes through unchanged.
//!
//! This is a pure state machine; it knows nothing about the terminal.

/// The `--escape-char` argument: an ASCII character, or `none` for no escapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::cli) struct EscapeChar(pub(in crate::cli) Option<u8>);

/// Reads the value of `--escape-char`. The character must be ASCII and must not
/// be one of the bytes the sequences themselves use (`.`, `?`, carriage return,
/// line feed), because the sequences would become ambiguous.
pub(in crate::cli) fn parse_escape_char(value: &str) -> Result<EscapeChar, String> {
    if value == "none" {
        return Ok(EscapeChar(None));
    }
    let mut bytes = value.bytes();
    let (Some(byte), None) = (bytes.next(), bytes.next()) else {
        // One byte of a `str` is one ASCII character; anything longer is refused.
        return Err("the escape character must be one ASCII character, or `none`".to_string());
    };
    if matches!(byte, b'.' | b'?' | b'\r' | b'\n') {
        return Err(
            "the escape character cannot be `.`, `?`, a carriage return or a line feed".to_string(),
        );
    }
    Ok(EscapeChar(Some(byte)))
}

/// What the typed bytes turn into, in the order they happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Bytes for the remote terminal.
    Send(Vec<u8>),
    /// Show the list of sequences on the local terminal; nothing is sent.
    Help,
    /// Leave the session. The bytes typed after it are dropped.
    Leave,
}

/// The state between two reads of the typed bytes.
pub(super) struct EscapeFilter {
    escape: Option<u8>,
    /// At the start of the session and after a sent carriage return or line feed.
    after_newline: bool,
    /// The escape character was typed at the start of a line and is held back.
    pending: bool,
}

impl EscapeFilter {
    /// `escape` is the escape character; `None` makes the session transparent.
    pub(super) fn new(escape: Option<u8>) -> Self {
        Self {
            escape,
            after_newline: true,
            pending: false,
        }
    }

    /// Turns the bytes of one read into steps. A held escape character is not
    /// sent until the next byte decides what it means.
    pub(super) fn feed(&mut self, typed: &[u8]) -> Vec<Step> {
        let mut steps = Vec::new();
        let Some(escape) = self.escape else {
            if !typed.is_empty() {
                steps.push(Step::Send(typed.to_vec()));
            }
            return steps;
        };
        let mut send = Vec::with_capacity(typed.len());
        for &byte in typed {
            if self.pending {
                self.pending = false;
                match byte {
                    b'.' => {
                        flush(&mut steps, &mut send);
                        steps.push(Step::Leave);
                        return steps;
                    }
                    b'?' => {
                        // As in ssh, the help leaves the "after a newline" state alone.
                        flush(&mut steps, &mut send);
                        steps.push(Step::Help);
                    }
                    _ => {
                        if byte != escape {
                            send.push(escape);
                        }
                        send.push(byte);
                        self.after_newline = is_newline(byte);
                    }
                }
            } else if self.after_newline && byte == escape {
                self.pending = true;
            } else {
                send.push(byte);
                self.after_newline = is_newline(byte);
            }
        }
        flush(&mut steps, &mut send);
        steps
    }
}

fn is_newline(byte: u8) -> bool {
    byte == b'\r' || byte == b'\n'
}

fn flush(steps: &mut Vec<Step>, send: &mut Vec<u8>) {
    if !send.is_empty() {
        steps.push(Step::Send(std::mem::take(send)));
    }
}

/// The text `~?` shows (raw terminal: every line ends in carriage return and
/// line feed).
pub(super) fn help_text(escape: u8) -> String {
    let c = char::from(escape);
    format!(
        "\r\nSupported escape sequences (only at the start of a line):\r\n  {c}.  leave (owner: end the session; attached: detach)\r\n  {c}{c}  send {c}\r\n  {c}?  this help\r\n"
    )
}

#[cfg(test)]
mod tests {
    use super::{help_text, parse_escape_char, EscapeChar, EscapeFilter, Step};

    fn sent(steps: &[Step]) -> Vec<u8> {
        steps
            .iter()
            .flat_map(|step| match step {
                Step::Send(bytes) => bytes.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn all(filter: &mut EscapeFilter, parts: &[&[u8]]) -> Vec<Step> {
        parts
            .iter()
            .flat_map(|part| filter.feed(part))
            .collect::<Vec<_>>()
    }

    #[test]
    fn terminal_task_the_escape_character_counts_at_the_start_and_after_a_newline() {
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b"~."), [Step::Leave]);

        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(
            filter.feed(b"ls\r~."),
            [Step::Send(b"ls\r".to_vec()), Step::Leave]
        );
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(
            filter.feed(b"ls\n~."),
            [Step::Send(b"ls\n".to_vec()), Step::Leave]
        );

        // In the middle of a line the tilde is an ordinary byte.
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b"a~."), [Step::Send(b"a~.".to_vec())]);
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b" ~."), [Step::Send(b" ~.".to_vec())]);
    }

    #[test]
    fn terminal_task_two_escape_characters_send_one_and_other_bytes_send_both() {
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b"~~"), [Step::Send(b"~".to_vec())]);
        // The sent tilde is no start of a line: the next `~.` is plain text.
        assert_eq!(filter.feed(b"~."), [Step::Send(b"~.".to_vec())]);

        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b"~x"), [Step::Send(b"~x".to_vec())]);

        // `~` followed by a newline sends both and starts a new line.
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(
            filter.feed(b"~\r~."),
            [Step::Send(b"~\r".to_vec()), Step::Leave]
        );
    }

    #[test]
    fn terminal_task_the_help_is_local_and_keeps_the_start_of_the_line() {
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b"~?"), [Step::Help]);
        // Like ssh, the help does not end the "start of a line" state.
        assert_eq!(filter.feed(b"~."), [Step::Leave]);

        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(
            filter.feed(b"~?abc"),
            [Step::Help, Step::Send(b"abc".to_vec())]
        );

        let help = help_text(b'~');
        for line in [
            "~.  leave (owner: end the session; attached: detach)",
            "~~  send ~",
            "~?  this help",
        ] {
            assert!(help.contains(line), "missing {line:?} in {help:?}");
        }
        assert!(help.contains("\r\n"));
        assert!(help_text(b'#').contains("#.  leave"));
    }

    #[test]
    fn terminal_task_nothing_after_the_leave_sequence_is_sent() {
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b"~.abc"), [Step::Leave]);
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(
            filter.feed(b"x\r~.abc\r"),
            [Step::Send(b"x\r".to_vec()), Step::Leave]
        );
    }

    #[test]
    fn terminal_task_an_escape_character_at_the_end_of_a_read_waits_for_the_next() {
        // `~` then `.` in two reads.
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert_eq!(filter.feed(b"ls\r~"), [Step::Send(b"ls\r".to_vec())]);
        assert_eq!(filter.feed(b"."), [Step::Leave]);

        // `~` then `~`: one tilde.
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert!(filter.feed(b"~").is_empty());
        assert_eq!(filter.feed(b"~"), [Step::Send(b"~".to_vec())]);

        // `~` then another byte: both.
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert!(filter.feed(b"~").is_empty());
        assert_eq!(filter.feed(b"x"), [Step::Send(b"~x".to_vec())]);

        // `~` then `?` then more input.
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert!(filter.feed(b"~").is_empty());
        assert_eq!(filter.feed(b"?"), [Step::Help]);
        assert_eq!(filter.feed(b"ok"), [Step::Send(b"ok".to_vec())]);

        // An empty read changes nothing.
        let mut filter = EscapeFilter::new(Some(b'~'));
        assert!(filter.feed(b"~").is_empty());
        assert!(filter.feed(b"").is_empty());
        assert_eq!(filter.feed(b"."), [Step::Leave]);
    }

    #[test]
    fn terminal_task_reading_byte_by_byte_gives_the_same_result_as_reading_at_once() {
        let typed: &[u8] = b"~~a\r~?\r~xb\n~.never";
        let mut whole = EscapeFilter::new(Some(b'~'));
        let at_once = whole.feed(typed);
        let mut single = EscapeFilter::new(Some(b'~'));
        let pieces: Vec<&[u8]> = typed.chunks(1).collect();
        let by_byte = all(&mut single, &pieces);
        assert_eq!(sent(&at_once), sent(&by_byte));
        let kinds = |steps: &[Step]| {
            steps
                .iter()
                .filter(|step| !matches!(step, Step::Send(_)))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(&at_once), kinds(&by_byte));
        assert_eq!(kinds(&at_once), [Step::Help, Step::Leave]);
        assert_eq!(sent(&at_once), b"~a\r\r~xb\n".to_vec());
    }

    #[test]
    fn terminal_task_none_makes_the_session_transparent_and_another_character_works() {
        let mut filter = EscapeFilter::new(None);
        assert_eq!(filter.feed(b"~."), [Step::Send(b"~.".to_vec())]);
        assert_eq!(filter.feed(b"\r~?"), [Step::Send(b"\r~?".to_vec())]);
        assert!(filter.feed(b"").is_empty());

        let mut filter = EscapeFilter::new(Some(b'#'));
        assert_eq!(filter.feed(b"~."), [Step::Send(b"~.".to_vec())]);
        assert_eq!(
            filter.feed(b"\r#."),
            [Step::Send(b"\r".to_vec()), Step::Leave]
        );
        let mut filter = EscapeFilter::new(Some(b'#'));
        assert_eq!(filter.feed(b"##"), [Step::Send(b"#".to_vec())]);
    }

    #[test]
    fn terminal_task_the_escape_character_argument_takes_one_ascii_character_or_none() {
        assert_eq!(parse_escape_char("~"), Ok(EscapeChar(Some(b'~'))));
        assert_eq!(parse_escape_char("#"), Ok(EscapeChar(Some(b'#'))));
        assert_eq!(parse_escape_char("none"), Ok(EscapeChar(None)));
        for bad in [
            "", "ab", "~~", "none ", "NONE", ".", "?", "\n", "\r", "é", "日",
        ] {
            assert!(parse_escape_char(bad).is_err(), "{bad:?} must be refused");
        }
    }
}
