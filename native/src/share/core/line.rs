use std::io::{self, BufRead};

pub(super) const MAX_SIGNAL_LINE: usize = 256 * 1024;

pub(super) fn read_line_limited(
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
                "signal line too large",
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
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "signal line invalid utf8"))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::read_line_limited;
    use std::io::Cursor;

    #[test]
    fn bounded_reader_preserves_next_line() {
        let mut reader = Cursor::new(b"one\ntwo\n".as_slice());
        let mut line = String::new();
        assert_eq!(read_line_limited(&mut reader, &mut line, 8).unwrap(), 4);
        assert_eq!(line, "one\n");
        assert_eq!(read_line_limited(&mut reader, &mut line, 8).unwrap(), 4);
        assert_eq!(line, "two\n");
    }

    #[test]
    fn bounded_reader_rejects_oversized_line() {
        let mut reader = Cursor::new(b"abcdef\n".as_slice());
        let mut line = String::new();
        assert!(read_line_limited(&mut reader, &mut line, 4).is_err());
    }
}
