//! Plan 2026-11-14 P3, route pauses of the TCP pump (`verbindungen_task_tcp_*`,
//! P28): a pause holds the idle budget, and an operation that is pending when
//! the route pauses is kept, not polled during the pause and continued after
//! it. The route is the fixture `route_monitor::test_route::TestRoute` on a
//! real QUIC connection, paused and resumed by the test.
use super::{idle_active, shared_op};
use crate::share::route_monitor::{self, test_route::TestRoute};
use crate::share::route_policy::batches_route_task_tests::{allow, pair, runtime};
use crate::share::route_protocol::{self, active_now};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::sync::{oneshot, watch};

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn verbindungen_task_tcp_route_pause_holds_idle_time() {
    runtime().block_on(async {
        let (left, right, client, _target) = pair().await;
        let route = TestRoute::admitted(&client, left.id().to_string());
        let limit = Duration::from_millis(400);
        // The idle budget reads the active clock of the connection in scope.
        let (error, after_pause) = route_protocol::scope(client.clone(), async {
            let (_progress, clock) = watch::channel(active_now());
            let idle = idle_active(clock, limit);
            tokio::pin!(idle);
            route.pause();
            // Twice the budget passes on the wall clock while the route is paused.
            let paused = tokio::time::timeout(2 * limit, &mut idle).await;
            assert!(
                paused.is_err(),
                "the idle budget ended during a route pause"
            );
            route.resume();
            let resumed = std::time::Instant::now();
            let error = tokio::time::timeout(WAIT, &mut idle)
                .await
                .expect("idle ends after the pause");
            (error, resumed.elapsed())
        })
        .await;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        // The pause began with the whole budget left; it is still left after it.
        assert!(
            after_pause >= limit / 2,
            "the pause consumed idle budget: {after_pause:?} of {limit:?} left"
        );
        drop(route);
        client.close(0u32.into(), b"fixture complete");
        left.close().await;
        right.close().await;
    });
}

#[test]
fn verbindungen_task_tcp_route_pause_keeps_the_pending_operation() {
    runtime().block_on(async {
        let (left, right, client, _target) = pair().await;
        // The transport admits the validated path; the fixture is the route status.
        allow(&client).await;
        let route = TestRoute::admitted(&client, left.id().to_string());
        let mut status = route_monitor::subscribe(&client).expect("route of the fixture");
        let mut observed = tokio::time::Instant::now();
        let polls = AtomicUsize::new(0);
        let (finish, mut finished) = oneshot::channel::<u8>();
        // One operation that counts how often it is polled.
        let operation = std::future::poll_fn(|context| {
            polls.fetch_add(1, Ordering::Relaxed);
            Pin::new(&mut finished)
                .poll(context)
                .map(|value| value.map_err(io::Error::other))
        });
        let pending = shared_op(&client, &mut status, &mut observed, operation);
        tokio::pin!(pending);
        let short = Duration::from_millis(200);
        // Admitted route: the operation starts and waits for its result.
        assert!(tokio::time::timeout(short, &mut pending).await.is_err());
        assert!(polls.load(Ordering::Relaxed) >= 1);
        route.pause();
        // The pause is seen; from then on the operation is not polled, also
        // not when its result arrives.
        assert!(tokio::time::timeout(short, &mut pending).await.is_err());
        let polls_at_pause = polls.load(Ordering::Relaxed);
        finish.send(7).expect("the operation still waits");
        assert!(
            tokio::time::timeout(short, &mut pending).await.is_err(),
            "the operation ended during a route pause"
        );
        assert_eq!(
            polls.load(Ordering::Relaxed),
            polls_at_pause,
            "the operation ran during a route pause"
        );
        route.resume();
        // The same operation continues and delivers its result.
        let value = tokio::time::timeout(WAIT, &mut pending)
            .await
            .expect("the operation ends after the pause")
            .expect("result of the operation");
        assert_eq!(value, 7);
        drop(route);
        client.close(0u32.into(), b"fixture complete");
        left.close().await;
        right.close().await;
    });
}
