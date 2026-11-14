//! E21 grant channel: a caller cannot hold the channel past its deadline.
use super::*;

/// Hands out one byte per read, each after `pause`.
struct Trickle {
    bytes: Vec<u8>,
    at: usize,
    pause: Duration,
}

impl Read for Trickle {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        std::thread::sleep(self.pause);
        if out.is_empty() || self.at >= self.bytes.len() {
            return Ok(0);
        }
        out[0] = self.bytes[self.at];
        self.at += 1;
        Ok(1)
    }
}

fn status_frame() -> Vec<u8> {
    let mut frame = Vec::new();
    wire::send(&mut frame, &SelfRequest::Status).expect("frame");
    frame
}

#[test]
fn login_selbst_task_trickling_caller_is_cut_at_the_deadline() {
    // Every read is quick enough for a per-read timeout, the whole
    // request is not: the channel is free again after the deadline.
    let mut slow = Trickle {
        bytes: status_frame(),
        at: 0,
        pause: Duration::from_millis(40),
    };
    let started = Instant::now();
    let error = receive_within(&mut slow, Duration::from_millis(120))
        .expect_err("a trickling caller must not finish its request");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(2));
    // A caller that sends its frame at once is served as before.
    let mut quick = Trickle {
        bytes: status_frame(),
        at: 0,
        pause: Duration::ZERO,
    };
    assert_eq!(
        receive_within(&mut quick, REQUEST_DEADLINE).expect("request"),
        SelfRequest::Status
    );
    assert!(REQUEST_DEADLINE <= Duration::from_secs(5));
}

#[test]
fn login_selbst_task_untrusted_channel_is_unreachable_not_refused() {
    // A pipe or socket another process holds (the worker refuses a server
    // that is not the broker) or no broker at all: the watch asks again.
    for error in [
        io::Error::new(io::ErrorKind::PermissionDenied, "not served by the broker"),
        io::Error::from(io::ErrorKind::NotFound),
    ] {
        let mapped = channel_unreachable(error);
        assert_eq!(mapped.kind(), io::ErrorKind::ConnectionRefused);
        assert_ne!(mapped.kind(), io::ErrorKind::PermissionDenied);
    }
}
