//! The peer asked us to slow down: a rate limit, "too many requests" or "too
//! many connections". Carried inside `io::Error` so every layer can pass it on
//! unchanged, and told apart from permanent conditions such as a full disk or
//! an exhausted storage quota, which `io::ErrorKind` already names.
use std::fmt;
use std::io;
use std::time::Duration;

/// Congestion reported by a peer; `retry_after` when it named a delay.
#[derive(Debug)]
pub struct Congestion {
    pub retry_after: Option<Duration>,
    pub message: String,
}

impl fmt::Display for Congestion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Congestion {}

/// An `io::Error` that transfers treat as backpressure, not as a failure.
pub fn congestion_error(message: impl Into<String>, retry_after: Option<Duration>) -> io::Error {
    io::Error::other(Congestion {
        retry_after,
        message: message.into(),
    })
}

/// The congestion carried by `error`, if it is one.
pub fn congestion_of(error: &io::Error) -> Option<&Congestion> {
    error.get_ref()?.downcast_ref::<Congestion>()
}
