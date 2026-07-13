//! Raw inbound byte limiting for synchronous WebSocket streams.

use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::rate_limits::InboundByteRateLimiter;

/// Charges bytes as they leave the socket, before Tungstenite parses frame headers or payloads.
pub(super) struct WebSocketReadLimit<S> {
    inner: S,
    inbound_bytes: InboundByteRateLimiter,
}

impl<S> WebSocketReadLimit<S> {
    pub(super) fn new(inner: S) -> Self {
        Self {
            inner,
            inbound_bytes: InboundByteRateLimiter::new(),
        }
    }

    #[cfg(test)]
    fn with_fixed_budget(inner: S, bytes: usize) -> Self {
        Self {
            inner,
            inbound_bytes: InboundByteRateLimiter::fixed_burst(bytes),
        }
    }
}

impl WebSocketReadLimit<TcpStream> {
    pub(super) fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.inner.set_nonblocking(nonblocking)
    }

    pub(super) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner.set_read_timeout(timeout)
    }
}

impl<S: Read> Read for WebSocketReadLimit<S> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        if read == 0 || self.inbound_bytes.try_consume(read) {
            return Ok(read);
        }
        Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "signaling raw WebSocket input rate limit exceeded",
        ))
    }
}

impl<S: Write> Write for WebSocketReadLimit<S> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use tungstenite::protocol::{Role, WebSocket};

    use super::*;

    struct FrameByFrameIo {
        inbound: VecDeque<Vec<u8>>,
        outbound: Vec<u8>,
    }

    impl FrameByFrameIo {
        fn new(frames: impl IntoIterator<Item = Vec<u8>>) -> Self {
            Self {
                inbound: frames.into_iter().collect(),
                outbound: Vec::new(),
            }
        }
    }

    impl Read for FrameByFrameIo {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(frame) = self.inbound.pop_front() else {
                return Ok(0);
            };
            assert!(
                frame.len() <= buffer.len(),
                "test frame exceeds read buffer"
            );
            buffer[..frame.len()].copy_from_slice(&frame);
            Ok(frame.len())
        }
    }

    impl Write for FrameByFrameIo {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.outbound.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn fragmented_empty_continuations_exhaust_raw_budget_before_message_completion() {
        let frames = [
            masked_frame(false, 0x1, b"x", [1, 2, 3, 4]),
            masked_frame(false, 0x0, b"", [5, 6, 7, 8]),
            masked_frame(false, 0x0, b"", [9, 10, 11, 12]),
            masked_frame(true, 0x0, b"", [13, 14, 15, 16]),
        ];
        let allowed_wire_bytes = frames[..3].iter().map(Vec::len).sum();
        assert_eq!(allowed_wire_bytes, 19);

        let stream =
            WebSocketReadLimit::with_fixed_budget(FrameByFrameIo::new(frames), allowed_wire_bytes);
        let mut websocket = WebSocket::from_raw_socket(stream, Role::Server, None);
        let error = websocket
            .read()
            .expect_err("final empty continuation must exceed the raw-byte budget");

        match error {
            tungstenite::Error::Io(error) => {
                assert_eq!(error.kind(), ErrorKind::PermissionDenied);
                assert_eq!(
                    error.to_string(),
                    "signaling raw WebSocket input rate limit exceeded"
                );
            }
            other => panic!("expected raw-byte rate error, got {other:?}"),
        }
    }

    fn masked_frame(fin: bool, opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
        assert!(payload.len() <= 125);
        let mut frame = Vec::with_capacity(6 + payload.len());
        frame.push((u8::from(fin) << 7) | opcode);
        frame.push(0x80 | payload.len() as u8);
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % mask.len()]),
        );
        frame
    }
}
