use std::io::{self, BufRead, BufReader};
use std::net::TcpStream;
use std::time::Instant;

pub(crate) const MAX_JSON_LINE: usize = 256 * 1024;

pub(crate) fn read_line_limited(
    reader: &mut impl BufRead,
    line: &mut String,
    max: usize,
) -> io::Result<usize> {
    line.clear();
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }
        let take = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|pos| pos + 1)
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
    use super::{read_line_limited, MAX_JSON_LINE};
    use std::io::Cursor;

    #[test]
    fn bounded_reader_keeps_following_line_buffered() {
        let mut reader = Cursor::new(b"one\ntwo\n".as_slice());
        let mut line = String::new();
        assert_eq!(
            read_line_limited(&mut reader, &mut line, MAX_JSON_LINE).unwrap(),
            4
        );
        assert_eq!(line, "one\n");
        assert_eq!(
            read_line_limited(&mut reader, &mut line, MAX_JSON_LINE).unwrap(),
            4
        );
        assert_eq!(line, "two\n");
    }

    #[test]
    fn bounded_reader_rejects_oversized_line() {
        let mut reader = Cursor::new(b"abcdef\n".as_slice());
        let mut line = String::new();
        assert!(read_line_limited(&mut reader, &mut line, 4).is_err());
    }
}
