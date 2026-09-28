use std::io::{self, BufRead};
use std::time::{Duration, Instant};

pub(super) const MAX_SIGNAL_LINE: usize = 256 * 1024;

const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TURN: Duration = Duration::from_millis(500);

/// One decoder per TCP connection: a read-poll timeout is not a message
/// boundary, and can occur between the bytes of one UTF-8 character.
#[derive(Default)]
pub(super) struct SignalLineReader {
    bytes: Vec<u8>,
    started: Option<Instant>,
}

impl SignalLineReader {
    pub(super) fn read(
        &mut self,
        reader: &mut impl BufRead,
        max: usize,
    ) -> io::Result<Option<String>> {
        let turn = Instant::now();
        loop {
            if self.started.is_some_and(|started| started.elapsed() >= FRAME_TIMEOUT) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "signal frame stalled"));
            }
            if turn.elapsed() >= READ_TURN {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let available = match reader.fill_buf() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if available.is_empty() {
                return if self.bytes.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, "incomplete signal frame"))
                };
            }
            let take = available.iter().position(|byte| *byte == b'\n')
                .map(|index| index + 1).unwrap_or(available.len());
            if self.bytes.len().saturating_add(take) > max {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "signal line too large"));
            }
            self.started.get_or_insert_with(Instant::now);
            self.bytes.extend_from_slice(&available[..take]);
            reader.consume(take);
            if self.bytes.last() == Some(&b'\n') {
                self.started = None;
                return String::from_utf8(std::mem::take(&mut self.bytes))
                    .map(Some)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "signal line invalid utf8"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SignalLineReader;
    use std::io::Cursor;

    #[test]
    fn bounded_reader_preserves_next_line() {
        let mut reader = Cursor::new(b"one\ntwo\n".as_slice());
        let mut decoder = SignalLineReader::default();
        assert_eq!(decoder.read(&mut reader, 8).unwrap().unwrap(), "one\n");
        assert_eq!(decoder.read(&mut reader, 8).unwrap().unwrap(), "two\n");
    }

    #[test]
    fn bounded_reader_rejects_oversized_line() {
        let mut reader = Cursor::new(b"abcdef\n".as_slice());
        assert!(SignalLineReader::default().read(&mut reader, 4).is_err());
    }
}

#[cfg(test)]
#[path = "signal_line_task_tests.rs"]
mod task_tests;
