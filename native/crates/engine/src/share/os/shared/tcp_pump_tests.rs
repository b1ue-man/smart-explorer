//! Plan 2026-11-14 P3 tests (`verbindungen_task_tcp_*`): the copy budget,
//! buffer growth on full reads, half-close in both directions, idle expiry
//! with progress, presence while pumping, and one real QUIC pump on the
//! shared route subscription.
use super::tcp_copy_budget::{CopyBudget, CopyBuffer};
use super::{both, copy, idle_active, pump_with, Lane, PumpEnv, PumpRoute};
use crate::share::keepalive::interactive_presence::{InteractiveKind, InteractivePresence};
use crate::share::route_monitor;
use crate::share::route_policy::batches_route_task_tests::{pair, runtime};
use crate::share::route_protocol::{self, active_now};
use crate::share::tcp_forward::{TCP_COPY_BUDGET_BYTES, TCP_COPY_BYTES, TCP_COPY_BYTES_GROWN};
use crate::share::tcp_registry::TcpCounters;
use std::{
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream, ReadBuf, ReadHalf, WriteHalf,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

const WAIT: Duration = Duration::from_secs(10);

fn leak_budget(capacity: u64) -> &'static CopyBudget {
    Box::leak(Box::new(CopyBudget::new(capacity)))
}

fn leak_presence() -> &'static InteractivePresence {
    Box::leak(Box::new(InteractivePresence::new()))
}

/// A route that needs no monitor. Leaked so that the pump future may hold it.
fn plain_route() -> &'static PumpRoute {
    Box::leak(Box::new(PumpRoute::Plain))
}

fn env(
    budget: &'static CopyBudget,
    presence: &'static InteractivePresence,
    legacy: bool,
) -> PumpEnv {
    PumpEnv {
        presence,
        budget,
        legacy,
    }
}

/// A reader that returns at most `step` bytes per read, so no read fills a buffer.
struct Trickle {
    data: Vec<u8>,
    step: usize,
    at: usize,
}

impl AsyncRead for Trickle {
    fn poll_read(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let length = (this.data.len() - this.at)
            .min(this.step)
            .min(buffer.remaining());
        buffer.put_slice(&this.data[this.at..this.at + length]);
        this.at += length;
        Poll::Ready(Ok(()))
    }
}

/// Two duplex pairs: the local side of the pump with its peer, and the QUIC
/// side with its peer. The peers play the far ends.
struct Pipes {
    local_peer: DuplexStream,
    quic_peer: DuplexStream,
    local_read: ReadHalf<DuplexStream>,
    local_write: WriteHalf<DuplexStream>,
    quic_read: ReadHalf<DuplexStream>,
    quic_write: WriteHalf<DuplexStream>,
}

impl Pipes {
    fn new() -> Self {
        let (local_peer, local) = tokio::io::duplex(4096);
        let (quic_peer, quic) = tokio::io::duplex(4096);
        let (local_read, local_write) = tokio::io::split(local);
        let (quic_read, quic_write) = tokio::io::split(quic);
        Self {
            local_peer,
            quic_peer,
            local_read,
            local_write,
            quic_read,
            quic_write,
        }
    }
}

#[test]
fn verbindungen_task_tcp_budget_grants_growth_only_within_capacity() {
    let budget = CopyBudget::new(100);
    budget.hold(40);
    assert!(budget.try_grow(50));
    assert!(!budget.try_grow(20));
    assert_eq!(budget.held(), 90);
    budget.give_back(90);
    assert_eq!(budget.held(), 0);
    budget.give_back(10);
    assert_eq!(budget.held(), 0);
}

#[test]
fn verbindungen_task_tcp_buffer_grows_on_full_read_within_budget() {
    runtime().block_on(async {
        let data = vec![7u8; 100 * 1024];
        let budget = leak_budget(TCP_COPY_BUDGET_BYTES);
        let mut buffer = CopyBuffer::new(budget, true);
        let count = AtomicU64::new(0);
        let (progress, _clock) = watch::channel(active_now());
        let mut reader: &[u8] = &data;
        let mut sink: Vec<u8> = Vec::new();
        copy(
            &mut PumpRoute::Plain,
            &mut reader,
            &mut sink,
            &count,
            &progress,
            &mut buffer,
        )
        .await
        .expect("copy");
        assert_eq!(sink, data);
        assert_eq!(buffer.size(), TCP_COPY_BYTES_GROWN);
        assert_eq!(budget.held(), TCP_COPY_BYTES_GROWN as u64);
        drop(buffer);
        assert_eq!(budget.held(), 0);
    });
}

#[test]
fn verbindungen_task_tcp_buffer_stays_base_on_short_reads() {
    runtime().block_on(async {
        let data = vec![3u8; 100 * 1024];
        let budget = leak_budget(TCP_COPY_BUDGET_BYTES);
        let mut buffer = CopyBuffer::new(budget, true);
        let count = AtomicU64::new(0);
        let (progress, _clock) = watch::channel(active_now());
        let mut reader = Trickle {
            data: data.clone(),
            step: 1024,
            at: 0,
        };
        let mut sink: Vec<u8> = Vec::new();
        copy(
            &mut PumpRoute::Plain,
            &mut reader,
            &mut sink,
            &count,
            &progress,
            &mut buffer,
        )
        .await
        .expect("copy");
        assert_eq!(sink, data);
        assert_eq!(buffer.size(), TCP_COPY_BYTES);
        assert_eq!(budget.held(), TCP_COPY_BYTES as u64);
    });
}

#[test]
fn verbindungen_task_tcp_buffer_stays_base_without_free_budget() {
    runtime().block_on(async {
        let data = vec![5u8; 100 * 1024];
        // Room for the base buffer only: growth is refused.
        let budget = leak_budget(TCP_COPY_BYTES as u64);
        let mut buffer = CopyBuffer::new(budget, true);
        let count = AtomicU64::new(0);
        let (progress, _clock) = watch::channel(active_now());
        let mut reader: &[u8] = &data;
        let mut sink: Vec<u8> = Vec::new();
        copy(
            &mut PumpRoute::Plain,
            &mut reader,
            &mut sink,
            &count,
            &progress,
            &mut buffer,
        )
        .await
        .expect("copy");
        assert_eq!(sink, data);
        assert_eq!(buffer.size(), TCP_COPY_BYTES);
        assert_eq!(budget.held(), TCP_COPY_BYTES as u64);
    });
}

#[test]
fn verbindungen_task_tcp_half_close_both_ways() {
    runtime().block_on(async {
        let budget = leak_budget(TCP_COPY_BUDGET_BYTES);
        let presence = leak_presence();
        let mut pipes = Pipes::new();
        let Pipes {
            local_peer,
            quic_peer,
            local_read,
            local_write,
            quic_read,
            quic_write,
        } = &mut pipes;
        let (sent, received) = (AtomicU64::new(0), AtomicU64::new(0));
        let progress = watch::channel(active_now()).0;
        let up = Lane {
            read: local_read,
            write: quic_write,
            count: &sent,
        };
        let down = Lane {
            read: quic_read,
            write: local_write,
            count: &received,
        };
        let pumped = both(
            plain_route(),
            env(budget, presence, false),
            &progress,
            up,
            down,
        );
        let client = async {
            local_peer.write_all(b"ping").await.expect("write ping");
            local_peer.shutdown().await.expect("half-close local side");
            let mut at_quic = Vec::new();
            quic_peer
                .read_to_end(&mut at_quic)
                .await
                .expect("read at quic side");
            quic_peer.write_all(b"pong").await.expect("write pong");
            quic_peer.shutdown().await.expect("half-close quic side");
            let mut at_local = Vec::new();
            local_peer
                .read_to_end(&mut at_local)
                .await
                .expect("read at local side");
            (at_quic, at_local)
        };
        let (result, (at_quic, at_local)) = tokio::join!(pumped, client);
        result.expect("pump ends after both half-closes");
        assert_eq!(at_quic, b"ping");
        assert_eq!(at_local, b"pong");
        assert_eq!(sent.load(Ordering::Relaxed), 4);
        assert_eq!(received.load(Ordering::Relaxed), 4);
        assert_eq!(budget.held(), 0);
    });
}

#[test]
fn verbindungen_task_tcp_presence_counts_while_pumping() {
    runtime().block_on(async {
        let budget = leak_budget(TCP_COPY_BUDGET_BYTES);
        let presence = leak_presence();
        let mut pipes = Pipes::new();
        let Pipes {
            local_peer,
            quic_peer,
            local_read,
            local_write,
            quic_read,
            quic_write,
        } = &mut pipes;
        let (sent, received) = (AtomicU64::new(0), AtomicU64::new(0));
        let progress = watch::channel(active_now()).0;
        let up = Lane {
            read: local_read,
            write: quic_write,
            count: &sent,
        };
        let down = Lane {
            read: quic_read,
            write: local_write,
            count: &received,
        };
        let pumped = both(
            plain_route(),
            env(budget, presence, false),
            &progress,
            up,
            down,
        );
        let client = async {
            // Yielding lets the pump run its first poll, which registers it.
            tokio::task::yield_now().await;
            assert_eq!(presence.count(InteractiveKind::TcpPump), 1);
            local_peer.shutdown().await.expect("half-close local side");
            quic_peer.shutdown().await.expect("half-close quic side");
        };
        let (result, ()) = tokio::join!(pumped, client);
        result.expect("pump ends");
        assert_eq!(presence.count(InteractiveKind::TcpPump), 0);
    });
}

#[test]
fn verbindungen_task_tcp_budget_returns_after_end() {
    runtime().block_on(async {
        let budget = leak_budget(TCP_COPY_BUDGET_BYTES);
        let presence = leak_presence();
        let mut pipes = Pipes::new();
        let Pipes {
            local_peer,
            quic_peer,
            local_read,
            local_write,
            quic_read,
            quic_write,
        } = &mut pipes;
        let (sent, received) = (AtomicU64::new(0), AtomicU64::new(0));
        let progress = watch::channel(active_now()).0;
        let up = Lane {
            read: local_read,
            write: quic_write,
            count: &sent,
        };
        let down = Lane {
            read: quic_read,
            write: local_write,
            count: &received,
        };
        let pumped = both(
            plain_route(),
            env(budget, presence, false),
            &progress,
            up,
            down,
        );
        let client = async {
            local_peer.shutdown().await.expect("half-close local side");
            quic_peer.shutdown().await.expect("half-close quic side");
        };
        let (result, ()) = tokio::join!(pumped, client);
        result.expect("pump ends");
        assert_eq!(budget.held(), 0);
    });
}

#[test]
fn verbindungen_task_tcp_idle_expires_without_progress() {
    runtime().block_on(async {
        let (_progress, clock) = watch::channel(active_now());
        let error = tokio::time::timeout(WAIT, idle_active(clock, Duration::from_millis(50)))
            .await
            .expect("idle ends within the wait");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    });
}

#[test]
fn verbindungen_task_tcp_idle_resets_with_progress() {
    runtime().block_on(async {
        let limit = Duration::from_millis(500);
        let (progress, clock) = watch::channel(active_now());
        let idle = idle_active(clock, limit);
        tokio::pin!(idle);
        // Operations complete for three times the idle budget: without the
        // reset by progress the budget would end after the first third.
        let started = std::time::Instant::now();
        let operations = async {
            while started.elapsed() < 3 * limit {
                tokio::time::sleep(Duration::from_millis(10)).await;
                progress.send_replace(active_now());
            }
        };
        tokio::select! {
            () = operations => {}
            _ = &mut idle => panic!("idle expired while operations completed"),
        }
        assert!(started.elapsed() >= 3 * limit);
        let error = tokio::time::timeout(WAIT, idle)
            .await
            .expect("idle ends after progress stopped");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    });
}

#[test]
fn verbindungen_task_tcp_quic_pump_drains_both_halves_under_presence_and_budget() {
    runtime().block_on(async {
        let (left, right, client, target) = pair().await;
        route_monitor::start(&client, left.id().to_string(), false, true, Vec::new()).unwrap();
        route_monitor::start(&target, right.id().to_string(), false, true, Vec::new()).unwrap();
        let deadline = Some(std::time::Instant::now() + WAIT);
        route_monitor::wait_application_route(&client, deadline)
            .await
            .unwrap();
        route_monitor::wait_application_route(&target, deadline)
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (connected, accepted) = tokio::join!(TcpStream::connect(address), listener.accept());
        let mut service = connected.unwrap();
        let (mut socket, _) = accepted.unwrap();
        let (mut outgoing, mut incoming) = client.open_bi().await.unwrap();
        outgoing.write_all(b"request").await.unwrap();
        outgoing.finish().unwrap();
        let (mut send, mut recv) = tokio::time::timeout(WAIT, target.accept_bi())
            .await
            .unwrap()
            .unwrap();
        let counters = Arc::new(TcpCounters::default());
        let presence = leak_presence();
        let budget = leak_budget(TCP_COPY_BUDGET_BYTES);
        let pumped = route_protocol::scope(
            target.clone(),
            pump_with(
                &target,
                &mut socket,
                &mut send,
                &mut recv,
                counters.clone(),
                env(budget, presence, false),
            ),
        );
        let service_side = async {
            let mut request = [0u8; 7];
            service.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"request");
            // The request is forwarded and the response is still outstanding,
            // so the pump is running.
            assert_eq!(presence.count(InteractiveKind::TcpPump), 1);
            service.write_all(b"response").await.unwrap();
            service.shutdown().await.unwrap();
            let mut rest = Vec::new();
            service.read_to_end(&mut rest).await.unwrap();
            rest
        };
        let client_side = async { incoming.read_to_end(1024).await.unwrap() };
        let (pump_result, rest, response) = tokio::time::timeout(WAIT, async {
            tokio::join!(pumped, service_side, client_side)
        })
        .await
        .expect("both halves drain in time");
        pump_result.expect("pump ends after both half-closes");
        assert_eq!(response, b"response");
        assert!(rest.is_empty());
        assert_eq!(counters.read.load(Ordering::Relaxed), 8);
        assert_eq!(counters.written.load(Ordering::Relaxed), 7);
        assert_eq!(counters.active.load(Ordering::Relaxed), 0);
        assert_eq!(presence.count(InteractiveKind::TcpPump), 0);
        assert_eq!(budget.held(), 0);
        client.close(0u32.into(), b"fixture complete");
        left.close().await;
        right.close().await;
    });
}
