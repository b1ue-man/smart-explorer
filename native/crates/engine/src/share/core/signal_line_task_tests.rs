use super::*;
use std::collections::VecDeque;
use std::io::{BufRead, Cursor, Read};

struct Fragments(VecDeque<io::Result<Cursor<Vec<u8>>>>);
impl Read for Fragments {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let n = output.len().min(available.len());
        output[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}
impl BufRead for Fragments {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        loop {
            let empty = self.0.front().is_some_and(|part| part.as_ref().is_ok_and(|part|
                part.position() as usize == part.get_ref().len()));
            if empty { self.0.pop_front(); continue; }
            if self.0.front().is_some_and(Result::is_err) {
                return Err(self.0.pop_front().unwrap().unwrap_err());
            }
            return match self.0.front_mut() {
                Some(Ok(part)) => part.fill_buf(),
                _ => Ok(&[]),
            };
        }
    }
    fn consume(&mut self, amount: usize) {
        if let Some(Ok(part)) = self.0.front_mut() { part.consume(amount); }
    }
}

#[test]
fn windows_remote_task_signal_frames_survive_timeout_and_split_utf8() {
    let bytes = "{\"name\":\"Grüße\"}\n{\"t\":\"pong\"}\n".as_bytes();
    let split = bytes.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
    let mut reader = Fragments(VecDeque::from([
        Ok(Cursor::new(bytes[..split].to_vec())),
        Err(io::ErrorKind::TimedOut.into()),
        Err(io::ErrorKind::WouldBlock.into()),
        Ok(Cursor::new(bytes[split..].to_vec())),
    ]));
    let mut decoder = SignalLineReader::default();
    assert_eq!(decoder.read(&mut reader, MAX_SIGNAL_LINE).unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert_eq!(decoder.read(&mut reader, MAX_SIGNAL_LINE).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert_eq!(decoder.read(&mut reader, MAX_SIGNAL_LINE).unwrap().unwrap(), "{\"name\":\"Grüße\"}\n");
    assert_eq!(decoder.read(&mut reader, MAX_SIGNAL_LINE).unwrap().unwrap(), "{\"t\":\"pong\"}\n");
    assert!(decoder.read(&mut reader, MAX_SIGNAL_LINE).unwrap().is_none());
}

#[test]
fn windows_remote_task_signal_frames_reject_truncation_limits_and_stalls() {
    let mut decoder = SignalLineReader::default();
    assert_eq!(decoder.read(&mut Cursor::new(b"partial"), 32).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    let mut decoder = SignalLineReader::default();
    let mut reader = Fragments(VecDeque::from([
        Ok(Cursor::new(b"1234".to_vec())), Err(io::ErrorKind::TimedOut.into()),
        Ok(Cursor::new(b"5\n".to_vec())),
    ]));
    assert_eq!(decoder.read(&mut reader, 4).unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert_eq!(decoder.read(&mut reader, 4).unwrap_err().kind(), io::ErrorKind::InvalidData);
    decoder.started = Some(Instant::now() - FRAME_TIMEOUT);
    assert_eq!(decoder.read(&mut Cursor::new(b"\n"), 32).unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert_eq!(SignalLineReader::default().read(&mut Cursor::new([0xff, b'\n']), 32).unwrap_err().kind(), io::ErrorKind::InvalidData);
}
