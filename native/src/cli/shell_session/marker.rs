//! End markers of session commands. After each command the shell prints
//! `\x1eSE<nonce>:<seq>:<exit>:<cwd>\x1e` on stdout and `\x1eSE<nonce>:<seq>\x1e`
//! on stderr; the command is done when both arrived. The nonce is random per
//! session, so command output does not produce a marker by accident. Bytes
//! that could start a marker at the end of a chunk are held back until the
//! next chunk decides.
const RECORD_SEPARATOR: u8 = 0x1e;
/// Longest marker body (sequence number, exit status, working directory); a
/// longer candidate is output. This bounds what a scanner holds back.
const MAX_BODY: usize = 8 * 1024;

/// 16 random hexadecimal characters.
pub(super) fn nonce() -> Result<String, String> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("cannot create the session nonce: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Piece {
    Data(Vec<u8>),
    /// The text between the marker prefix and its closing separator.
    Marker(String),
}

/// Splits one output stream into command output and marker bodies.
pub(super) struct MarkerScanner {
    prefix: Vec<u8>,
    pending: Vec<u8>,
}

impl MarkerScanner {
    pub(super) fn new(nonce: &str) -> Self {
        let mut prefix = vec![RECORD_SEPARATOR];
        prefix.extend_from_slice(b"SE");
        prefix.extend_from_slice(nonce.as_bytes());
        prefix.push(b':');
        Self {
            prefix,
            pending: Vec::new(),
        }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        self.pending.extend_from_slice(bytes);
        let mut pieces = Vec::new();
        loop {
            let Some(start) = find(&self.pending, &self.prefix) else {
                let keep = partial_prefix(&self.pending, &self.prefix);
                let emit = self.pending.len() - keep;
                if emit > 0 {
                    pieces.push(Piece::Data(self.pending.drain(..emit).collect()));
                }
                break;
            };
            if start > 0 {
                pieces.push(Piece::Data(self.pending.drain(..start).collect()));
            }
            let body_start = self.prefix.len();
            let closing = self.pending[body_start..]
                .iter()
                .position(|byte| *byte == RECORD_SEPARATOR);
            match closing {
                Some(length) => {
                    let body =
                        String::from_utf8_lossy(&self.pending[body_start..body_start + length])
                            .into_owned();
                    self.pending.drain(..body_start + length + 1);
                    pieces.push(Piece::Marker(body));
                }
                None if self.pending.len() - body_start > MAX_BODY => {
                    pieces.push(Piece::Data(self.pending.drain(..1).collect()));
                }
                None => break,
            }
        }
        pieces
    }

    /// Bytes still held back; the stream has ended.
    pub(super) fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

/// `<seq>:<exit>:<cwd>`; the exit status is `None` when the shell printed none.
pub(super) fn parse_stdout(body: &str) -> Option<(u64, Option<i32>, String)> {
    let mut parts = body.splitn(3, ':');
    let seq = parts.next()?.parse().ok()?;
    let exit = parts.next()?.trim().parse().ok();
    let cwd = parts.next()?.to_string();
    Some((seq, exit, cwd))
}

pub(super) fn parse_stderr(body: &str) -> Option<u64> {
    body.parse().ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Length of the longest end of `bytes` that is a proper start of `prefix`.
fn partial_prefix(bytes: &[u8], prefix: &[u8]) -> usize {
    let longest = prefix.len().saturating_sub(1).min(bytes.len());
    (1..=longest)
        .rev()
        .find(|length| bytes[bytes.len() - length..] == prefix[..*length])
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{nonce, parse_stderr, parse_stdout, MarkerScanner, Piece};

    #[test]
    fn shell_session_task_markers_split_output_and_hold_partial_prefixes() {
        let mut scanner = MarkerScanner::new("0123456789abcdef");
        assert_eq!(
            scanner.feed(b"hello\x1eSE0123"),
            vec![Piece::Data(b"hello".to_vec())]
        );
        assert_eq!(
            scanner.feed(b"456789abcdef:1:0:/tmp/a:b\x1etail"),
            vec![
                Piece::Marker("1:0:/tmp/a:b".into()),
                Piece::Data(b"tail".to_vec())
            ]
        );
        // A record separator that starts no marker is output.
        assert_eq!(
            scanner.feed(b"x\x1ey"),
            vec![Piece::Data(b"x\x1ey".to_vec())]
        );
        // The marker of another session (other nonce) is output too; only
        // its last byte waits, because it could start a marker.
        let foreign = b"\x1eSEffffffffffffffff:1\x1e";
        assert_eq!(
            scanner.feed(foreign),
            vec![Piece::Data(foreign[..foreign.len() - 1].to_vec())]
        );
        assert_eq!(
            scanner.feed(b"z\x1eS"),
            vec![Piece::Data(b"\x1ez".to_vec())]
        );
        assert_eq!(scanner.flush(), b"\x1eS".to_vec());
        // A candidate without its closing separator stays output once it
        // is longer than any marker.
        let mut scanner = MarkerScanner::new("00");
        let mut long = b"\x1eSE00:".to_vec();
        long.extend(std::iter::repeat_n(b'a', 9 * 1024));
        let pieces = scanner.feed(&long);
        let data: usize = pieces
            .iter()
            .map(|piece| match piece {
                Piece::Data(bytes) => bytes.len(),
                Piece::Marker(_) => 0,
            })
            .sum();
        assert_eq!(data, long.len());
    }

    #[test]
    fn shell_session_task_marker_bodies_parse_exit_and_cwd() {
        assert_eq!(
            parse_stdout("7:3:C:\\Users\\a b"),
            Some((7, Some(3), "C:\\Users\\a b".to_string()))
        );
        assert_eq!(
            parse_stdout("2:-1073741819:/"),
            Some((2, Some(-1_073_741_819), "/".to_string()))
        );
        assert_eq!(parse_stdout("2::/srv"), Some((2, None, "/srv".to_string())));
        assert_eq!(parse_stdout("x:0:/"), None);
        assert_eq!(parse_stderr("12"), Some(12));
        assert_eq!(parse_stderr("12:0"), None);
        let nonce = nonce().unwrap();
        assert_eq!(nonce.len(), 16);
        assert!(nonce.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
