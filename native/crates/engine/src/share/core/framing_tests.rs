//! Payload reads of `framing` (review r-weg-1): a head that announces a large
//! frame costs memory only for the bytes that arrive, a real large frame is
//! read byte-equal, and a cut stream ends with `UnexpectedEof`.
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, ReadBuf};

use super::super::fs::CHUNK;
use super::*;

const WAIT: Duration = Duration::from_secs(20);
const MIB: usize = 1024 * 1024;
/// Allocation granularity a `reserve_exact` may round up by.
const ROUNDING: usize = 4096;

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

/// A reader that gives at most `step` bytes per read and remembers the
/// widest room any read was offered.
struct Trickle<'a> {
    data: &'a [u8],
    step: usize,
    widest: usize,
}

impl AsyncRead for Trickle<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.widest = this.widest.max(buf.remaining());
        let count = this.data.len().min(this.step).min(buf.remaining());
        buf.put_slice(&this.data[..count]);
        this.data = &this.data[count..];
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn framing_task_announced_16_mib_head_with_a_cut_stream_reserves_one_chunk() {
    // E1: three bytes arrive, then the stream ends.
    let error = tokio::time::timeout(WAIT, read_payload(&mut &[1u8, 2, 3][..], MAX_FRAME))
        .await
        .expect("read deadline")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(error.to_string(), "peer stream closed after 3 bytes");
    // No read was ever offered room for the announced 16 MiB.
    let mut reader = Trickle {
        data: &[1, 2, 3],
        step: usize::MAX,
        widest: 0,
    };
    let error = tokio::time::timeout(WAIT, read_payload(&mut reader, MAX_FRAME))
        .await
        .expect("read deadline")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert!(reader.widest > 0, "the payload read took the stream");
    assert!(
        reader.widest < 2 * PAYLOAD_FIRST_RESERVE,
        "room offered for the first read: {}",
        reader.widest
    );
    // The first room of a payload that announces 16 MiB is one content frame.
    let mut payload: Vec<u8> = Vec::new();
    reserve_payload(&mut payload, MAX_FRAME);
    assert!(payload.capacity() >= PAYLOAD_FIRST_RESERVE);
    assert!(
        payload.capacity() < 2 * PAYLOAD_FIRST_RESERVE,
        "capacity {}",
        payload.capacity()
    );
}

#[test]
fn framing_task_full_payload_buffer_grows_by_its_own_capacity_never_beyond_the_frame() {
    // E2: a full buffer of the first reserve doubles towards a 1 MiB frame.
    let mut payload: Vec<u8> = Vec::with_capacity(PAYLOAD_FIRST_RESERVE);
    payload.resize(payload.capacity(), 0);
    let full = payload.capacity();
    reserve_payload(&mut payload, MIB);
    assert!(
        payload.capacity() >= 2 * PAYLOAD_FIRST_RESERVE,
        "capacity {}",
        payload.capacity()
    );
    assert!(payload.capacity() >= 2 * full);
    assert!(
        payload.capacity() <= MIB + ROUNDING,
        "capacity {}",
        payload.capacity()
    );
    // A single byte left grows the buffer by that byte, not by a doubling.
    let mut payload: Vec<u8> = Vec::with_capacity(PAYLOAD_FIRST_RESERVE);
    payload.resize(payload.capacity(), 0);
    let full = payload.capacity();
    reserve_payload(&mut payload, full + 1);
    let growth = payload.capacity() - full;
    assert!(growth >= 1, "growth {growth}");
    assert!(growth < PAYLOAD_FIRST_RESERVE, "growth {growth}");
    // A buffer with room left is not touched.
    let mut payload: Vec<u8> = Vec::with_capacity(PAYLOAD_FIRST_RESERVE);
    payload.extend_from_slice(&[7; 10]);
    let before = payload.capacity();
    reserve_payload(&mut payload, MAX_FRAME);
    assert_eq!(payload.capacity(), before);
}

#[tokio::test]
async fn framing_task_payloads_arrive_byte_equal_at_every_buffer_boundary() {
    // E3: empty, around the first reserve and its doubling, one chunk, one
    // data frame, 1 MiB; whole and in small pieces. Seven bytes of the next
    // frame stay in the stream.
    let sizes = [
        0,
        1,
        PAYLOAD_FIRST_RESERVE - 1,
        PAYLOAD_FIRST_RESERVE,
        PAYLOAD_FIRST_RESERVE + 1,
        2 * PAYLOAD_FIRST_RESERVE,
        2 * PAYLOAD_FIRST_RESERVE + 1,
        CHUNK,
        MAX_DATA_FRAME,
        MAX_DATA_FRAME_LZ4,
        MIB,
    ];
    for step in [usize::MAX, 1000] {
        for size in sizes {
            let stream = pattern(size + 7);
            let mut reader = Trickle {
                data: &stream,
                step,
                widest: 0,
            };
            let payload = tokio::time::timeout(WAIT, read_payload(&mut reader, size))
                .await
                .expect("read deadline")
                .unwrap_or_else(|error| panic!("size {size}, step {step}: {error}"));
            assert_eq!(payload.len(), size);
            assert!(payload[..] == stream[..size], "size {size}, step {step}");
            assert_eq!(reader.data, &stream[size..], "size {size}, step {step}");
        }
    }
}

#[test]
fn framing_task_control_frame_cut_after_three_bytes_ends_with_unexpected_eof() {
    use super::super::route_policy::batches_route_task_tests::{allow, pair, runtime};
    // E4 over a real loopback connection: the head announces 16 MiB, three
    // bytes follow, the sender ends the stream.
    runtime().block_on(async {
        let (left, right, sender, receiver) = pair().await;
        allow(&sender).await;
        let mut send = sender.open_uni().await.unwrap();
        send.write_all(&[0x01, 0x00, 0x00, 0x00, TAG_CTRL])
            .await
            .unwrap();
        send.write_all(&[1, 2, 3]).await.unwrap();
        send.finish().unwrap();
        let mut recv = tokio::time::timeout(WAIT, receiver.accept_uni())
            .await
            .expect("accept deadline")
            .unwrap();
        let error = tokio::time::timeout(WAIT, recv_ctrl(&mut recv))
            .await
            .expect("receive deadline")
            .err()
            .expect("a cut control frame is an error");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(error.to_string(), "peer stream closed after 3 bytes");
        sender.close(0u32.into(), b"framing task done");
        left.close().await;
        right.close().await;
    });
}
