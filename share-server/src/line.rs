use std::io::{self, BufRead, BufReader};
use std::net::TcpStream;
use std::time::Instant;

pub(crate) const MAX_JSON_LINE: usize = 256 * 1024;

/// Result of one [`LineReader::read`] call.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LineRead {
    Line(String),
    Eof,
}

/// Assembles bounded newline-delimited lines. A read timeout or interruption
/// returns the error but keeps the partial line, so the next call continues
/// it; connection loops can therefore wake up for keepalive deadlines
/// without losing input.
#[derive(Default)]
pub(crate) struct LineReader {
    partial: Vec<u8>,
    received: bool,
}

impl LineReader {
    pub(crate) fn read(&mut self, reader: &mut impl BufRead, max: usize) -> io::Result<LineRead> {
        loop {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                // An unterminated final line is still delivered, as before.
                if self.partial.is_empty() {
                    return Ok(LineRead::Eof);
                }
                return self.finish();
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|position| position + 1)
                .unwrap_or(available.len());
            if self.partial.len().saturating_add(take) > max {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "json line too large",
                ));
            }
            self.partial.extend_from_slice(&available[..take]);
            reader.consume(take);
            self.received = true;
            if self.partial.last() == Some(&b'\n') {
                return self.finish();
            }
        }
    }

    /// Whether any bytes arrived since the previous call, complete line or not.
    pub(crate) fn take_received(&mut self) -> bool {
        std::mem::take(&mut self.received)
    }

    fn finish(&mut self) -> io::Result<LineRead> {
        String::from_utf8(std::mem::take(&mut self.partial))
            .map(LineRead::Line)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "json line invalid utf8"))
    }
}

pub(crate) fn read_line_limited_until(
    reader: &mut BufReader<TcpStream>,
    line: &mut String,
    max: usize,
    deadline: Instant,
) -> io::Result<usize> {
    line.clear();
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(deadline_expired());
        }
        reader.get_ref().set_read_timeout(Some(remaining))?;
        let available = match reader.fill_buf() {
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                return Err(deadline_expired());
            }
            Err(error) => return Err(error),
            Ok(available) => available,
        };
        if available.is_empty() {
            break;
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| position + 1)
            .unwrap_or(available.len());
        if bytes.len().saturating_add(take) > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "json line too large",
            ));
        }
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if bytes.last() == Some(&b'\n') {
            break;
        }
    }
    let n = bytes.len();
    *line = String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "json line invalid utf8"))?;
    Ok(n)
}

fn deadline_expired() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "pre-registration deadline expired")
}

#[cfg(test)]
mod tests {
    use super::{LineRead, LineReader, MAX_JSON_LINE};
    use std::io::{self, BufReader, Cursor, Read};

    #[test]
    fn bounded_reader_keeps_following_line_buffered() {
        let mut reader = Cursor::new(b"one\ntwo\n".as_slice());
        let mut lines = LineReader::default();
        assert_eq!(
            lines.read(&mut reader, MAX_JSON_LINE).unwrap(),
            LineRead::Line("one\n".into())
        );
        assert_eq!(
            lines.read(&mut reader, MAX_JSON_LINE).unwrap(),
            LineRead::Line("two\n".into())
        );
        assert_eq!(
            lines.read(&mut reader, MAX_JSON_LINE).unwrap(),
            LineRead::Eof
        );
    }

    #[test]
    fn bounded_reader_rejects_oversized_line() {
        let mut reader = Cursor::new(b"abcdef\n".as_slice());
        assert!(LineReader::default().read(&mut reader, 4).is_err());
    }

    /// Yields its chunks one read at a time and times out between them.
    struct TimedChunks(Vec<Option<&'static [u8]>>);

    impl Read for TimedChunks {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            match self.0.remove(0) {
                Some(chunk) => {
                    buffer[..chunk.len()].copy_from_slice(chunk);
                    Ok(chunk.len())
                }
                None => Err(io::Error::new(io::ErrorKind::TimedOut, "read timeout")),
            }
        }
    }

    #[test]
    fn android_background_task_partial_line_survives_read_timeouts() {
        let mut reader = BufReader::new(TimedChunks(vec![
            Some(b"{\"t\":\"heart"),
            None,
            None,
            Some(b"beat\"}\n{\"t\""),
            None,
            Some(b":\"keepalive_ack\"}"),
        ]));
        let mut lines = LineReader::default();
        let timeout = lines.read(&mut reader, MAX_JSON_LINE).unwrap_err();
        assert_eq!(timeout.kind(), io::ErrorKind::TimedOut);
        assert!(lines.take_received(), "partial bytes count as activity");
        assert!(lines.read(&mut reader, MAX_JSON_LINE).is_err());
        assert!(!lines.take_received());
        assert_eq!(
            lines.read(&mut reader, MAX_JSON_LINE).unwrap(),
            LineRead::Line("{\"t\":\"heartbeat\"}\n".into())
        );
        assert!(lines.read(&mut reader, MAX_JSON_LINE).is_err());
        assert_eq!(
            lines.read(&mut reader, MAX_JSON_LINE).unwrap(),
            LineRead::Line("{\"t\":\"keepalive_ack\"}".into())
        );
        assert_eq!(
            lines.read(&mut reader, MAX_JSON_LINE).unwrap(),
            LineRead::Eof
        );
    }
}
