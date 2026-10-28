//! Sized copy stages and copied errors of the client transfer path.
use super::*;

struct Recorder(std::sync::Arc<Mutex<Vec<u8>>>, std::sync::Arc<Mutex<bool>>);

impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        *self.1.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
fn transfer_engine_task_sized_stage_commits_only_exact_length() {
    let bytes = std::sync::Arc::new(Mutex::new(Vec::new()));
    let flushed = std::sync::Arc::new(Mutex::new(false));
    let mut short = sized_writer(Box::new(Recorder(bytes.clone(), flushed.clone())), 5);
    short.write_all(b"abc").unwrap();
    assert_eq!(
        short.flush().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(
        !*flushed.lock().unwrap(),
        "a short stage is never committed"
    );
    assert_eq!(
        short.write(b"def").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    short.write_all(b"de").unwrap();
    short.flush().unwrap();
    assert!(*flushed.lock().unwrap());
    assert_eq!(*bytes.lock().unwrap(), b"abcde");

    let congestion = crate::vfs::congestion_error("voll", Some(Duration::from_secs(2)));
    let copied = copy_error(&congestion);
    let copied = crate::vfs::congestion_of(&copied).expect("congestion stays congestion");
    assert_eq!(copied.retry_after, Some(Duration::from_secs(2)));
    let plain = copy_error(&io::Error::new(io::ErrorKind::NotFound, "weg"));
    assert_eq!(plain.kind(), io::ErrorKind::NotFound);
    assert_eq!(plain.to_string(), "weg");
    assert_eq!(server_copy_budget(0), PEER_OP_TIMEOUT);
    assert_eq!(
        server_copy_budget(10 * MIN_SERVER_COPY_RATE),
        PEER_OP_TIMEOUT + Duration::from_secs(10)
    );
}
