//! Connects that find no free local address (G3, suite run 2 on Windows:
//! 10048 at STOR's data connection once the runner's ephemeral ports waited
//! in TIME_WAIT). Such a connect never reached the server, and the server
//! still waits for it: it is tried again while time is left, so a file does
//! not fail for want of a port that frees up a moment later.
use std::io;
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

/// The connect failed on this machine, not at the server: no free local
/// address (port) was left for a new connection. Windows answers that with
/// WSAEADDRINUSE (10048) while its ephemeral ports wait in TIME_WAIT, Linux
/// with EADDRNOTAVAIL. Ports come free again as their TIME_WAIT ends, so the
/// connect is tried again instead of failing the file.
pub(super) fn local_address_shortage(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::AddrInUse | io::ErrorKind::AddrNotAvailable
    )
}

/// First pause before a connect is tried again for want of a local address;
/// it doubles up to `SHORTAGE_PAUSE_MAX`. Ports leave TIME_WAIT one by one in
/// a busy transfer, so the first retry comes soon and later ones do not spin.
const SHORTAGE_PAUSE_MIN: Duration = Duration::from_millis(1);
const SHORTAGE_PAUSE_MAX: Duration = Duration::from_millis(100);

/// `connect` with the time left until `deadline`, tried again while it fails
/// for want of a free local address (`local_address_shortage`) and time is
/// left. Every other answer, and the shortage at the deadline, is returned.
pub(super) fn connect_until(
    deadline: Instant,
    mut connect: impl FnMut(Duration) -> io::Result<TcpStream>,
) -> io::Result<TcpStream> {
    let mut pause = SHORTAGE_PAUSE_MIN;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "FTP connect timed out",
            ));
        }
        match connect(left) {
            Err(error) if local_address_shortage(&error) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(error);
                }
                thread::sleep(pause.min(left));
                pause = (pause * 2).min(SHORTAGE_PAUSE_MAX);
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn speed_task_ftp_connect_waits_out_a_missing_local_address() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut attempts = 0;
        let stream = connect_until(Instant::now() + Duration::from_secs(5), |left| {
            attempts += 1;
            if attempts < 3 {
                return Err(io::Error::from(io::ErrorKind::AddrInUse));
            }
            TcpStream::connect_timeout(&address, left)
        })
        .expect("the third attempt finds a local address");
        assert_eq!(attempts, 3);
        drop(stream);
    }

    #[test]
    fn speed_task_ftp_connect_returns_other_errors_and_the_shortage_at_the_deadline() {
        let mut attempts = 0;
        let refused = connect_until(Instant::now() + Duration::from_secs(5), |_| {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::ConnectionRefused))
        })
        .unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::ConnectionRefused);
        assert_eq!(attempts, 1, "an answer of the server is not repeated");
        let short = connect_until(Instant::now() + Duration::from_millis(30), |_| {
            Err(io::Error::from(io::ErrorKind::AddrNotAvailable))
        })
        .unwrap_err();
        assert!(local_address_shortage(&short), "{short}");
    }
}
