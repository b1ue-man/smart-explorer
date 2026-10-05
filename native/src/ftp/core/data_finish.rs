//! Keep FTPS transport ownership through TLS Drop and the terminal FTP reply.
use std::io::{self, Write};
use std::net::TcpStream;
use suppaftp::{FtpError, RustlsFtpStream};

pub(super) struct UploadData {
    data: Box<dyn Write + Send>,
    retained: io::Result<Option<TcpStream>>,
}

impl UploadData {
    pub(super) fn new(
        data: impl Write + Send + 'static,
        retained: io::Result<Option<TcpStream>>,
    ) -> Self {
        Self {
            data: Box::new(data),
            retained,
        }
    }

    pub(super) fn finish(self, control: &mut RustlsFtpStream) -> io::Result<()> {
        let Self { mut data, retained } = self;
        let (socket, retained) = match retained {
            Ok(socket) => (socket, Ok(())),
            Err(error) => (None, Err(error)),
        };
        let flushed = data.flush();
        // suppaftp drops the verified TLS writer (sending close_notify) before
        // reading 226/250. Its socket must not become the last handle then:
        // unread TLS messages can otherwise turn the final TCP close into RST.
        // Plain FTP has no retained socket and still sends its actual EOF.
        let finished = control
            .finalize_put_stream(data)
            .map_err(super::errors::map);
        drop(socket);
        match finished {
            Err(error) if terminal_refusal(&error) => Err(error),
            finished => flushed.and(retained).and(finished),
        }
    }
}

impl Write for UploadData {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.data.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.data.flush()
    }
}

pub(super) fn terminal_refusal(error: &io::Error) -> bool {
    crate::vfs::is_target_refusal(error)
        || error
            .get_ref()
            .and_then(|source| source.downcast_ref::<FtpError>())
            .and_then(super::errors::reply_code)
            .is_some_and(|code| matches!(code, 451 | 452 | 552))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    struct BrokenData {
        dropped: mpsc::Sender<()>,
    }

    impl Write for BrokenData {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "data write"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "data flush"))
        }
    }

    impl Drop for BrokenData {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    #[test]
    fn sync_reliability_task_provider_upload_data_error_still_consumes_terminal_refusal() {
        for (code, kind) in [
            (451, io::ErrorKind::Other),
            (452, io::ErrorKind::StorageFull),
            (552, io::ErrorKind::QuotaExceeded),
            (426, io::ErrorKind::ConnectionReset),
            (226, io::ErrorKind::ConnectionReset),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let client = TcpStream::connect_timeout(
                &listener.local_addr().unwrap(),
                Duration::from_secs(5),
            )
            .unwrap();
            let (mut server, _) = listener.accept().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            client
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            server
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            server.write_all(b"220 Owned finish guard\r\n").unwrap();
            let mut control = RustlsFtpStream::connect_with_stream(client).unwrap();
            let (dropped, received) = mpsc::channel();
            let worker = thread::spawn(move || {
                // The terminal answer must be read after dropping data, even
                // when its flush failed; no TLS behavior is simulated here.
                if received.recv_timeout(Duration::from_secs(5)).is_ok() {
                    let _ = write!(server, "{code} Terminal fixture answer\r\n");
                }
            });
            let data = UploadData::new(BrokenData { dropped }, Ok(None));
            let result = data.finish(&mut control);
            worker.join().unwrap();
            let error = result.unwrap_err();
            assert_eq!(error.kind(), kind);
            if matches!(code, 451 | 452 | 552) {
                assert!(error.to_string().contains(&code.to_string()));
            } else {
                assert_eq!(error.to_string(), "data flush");
            }
        }
    }
}
