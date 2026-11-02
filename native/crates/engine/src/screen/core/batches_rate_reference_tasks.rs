//! Explicit shutdown quiesces measurement tasks before dropping channel owners.
use std::future::Future;
use std::io;

use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

enum Exit {
    Stopped,
    Returned(io::Result<()>),
}

struct Task {
    quiesced: oneshot::Receiver<()>,
    handle: JoinHandle<Exit>,
}

pub(super) struct Tasks {
    owned: Vec<Task>,
    stop: watch::Sender<()>,
    release: watch::Sender<()>,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            owned: Vec::new(),
            stop: watch::channel(()).0,
            release: watch::channel(()).0,
        }
    }
}

impl Tasks {
    pub fn spawn(&mut self, future: impl Future<Output = io::Result<()>> + Send + 'static) {
        let mut stop = self.stop.subscribe();
        let mut release = self.release.subscribe();
        let (ready, quiesced) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut future = Box::pin(future);
            // Check a ready real result before stopping. Keep pending futures
            // alive until EVERY task has left its measurement loop, so dropping
            // an ACK sender cannot manufacture a peer's early-closure error.
            let outcome = tokio::select! {
                biased;
                result = future.as_mut() => Exit::Returned(result),
                _ = stop.changed() => Exit::Stopped,
            };
            let _ = ready.send(());
            let _ = release.changed().await;
            drop(future);
            outcome
        });
        self.owned.push(Task { quiesced, handle });
    }

    pub async fn finish(mut self) -> io::Result<()> {
        self.stop.send_replace(());
        for task in &mut self.owned {
            // A panic closes this sender; its JoinError is still collected below.
            let _ = (&mut task.quiesced).await;
        }
        self.release.send_replace(());
        let mut failures = Vec::new();
        // Retain every handle in self while awaiting: dropping this finish
        // future must still abort every unjoined child, including the current one.
        for (index, task) in self.owned.iter_mut().enumerate() {
            match (&mut task.handle).await {
                Ok(Exit::Stopped) => {}
                Ok(Exit::Returned(result)) => {
                    failures.push(format!("measurement task {index} returned: {result:?}"))
                }
                Err(error) => failures.push(format!("measurement task {index}: {error}")),
            }
        }
        self.owned.clear();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(failures.join("; ")))
        }
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.owned {
            task.handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use std::task::Poll;
    use std::time::Duration;

    struct Dropped(Option<oneshot::Sender<()>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            if let Some(sent) = self.0.take() {
                let _ = sent.send(());
            }
        }
    }

    fn pending_child(tasks: &mut Tasks) -> oneshot::Receiver<()> {
        let (sent, received) = oneshot::channel();
        let guard = Dropped(Some(sent));
        tasks.spawn(async move {
            let _guard = guard;
            pending::<io::Result<()>>().await
        });
        received
    }

    fn run(future: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(5), future)
                    .await
                    .expect("owned task shutdown is bounded");
            });
    }

    #[test]
    fn p27_task_shutdown_preserves_ack_and_picture_dependencies_until_quiesced() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                let mut tasks = Tasks::default();
                let (ack_tx, mut ack_rx) = tokio::sync::mpsc::channel::<()>(1);
                let (picture_tx, mut picture_rx) = tokio::sync::mpsc::channel::<()>(1);
                let (started, mut ready) = tokio::sync::mpsc::channel(3);
                let producer_started = started.clone();
                tasks.spawn(async move {
                    let _ack_owner = ack_tx;
                    producer_started.send(()).await.unwrap();
                    pending::<io::Result<()>>().await
                });
                let actor_started = started.clone();
                tasks.spawn(async move {
                    let _picture_owner = picture_tx;
                    actor_started.send(()).await.unwrap();
                    ack_rx
                        .recv()
                        .await
                        .ok_or_else(|| io::Error::other("wire ACK stream closed"))?;
                    Err(io::Error::other("unexpected fixture ACK"))
                });
                tasks.spawn(async move {
                    started.send(()).await.unwrap();
                    picture_rx
                        .recv()
                        .await
                        .ok_or_else(|| io::Error::other("picture reader closed"))?;
                    Err(io::Error::other("unexpected fixture picture"))
                });
                for _ in 0..3 {
                    ready.recv().await.unwrap();
                }
                let dropped = pending_child(&mut tasks);
                tasks.finish().await.unwrap();
                dropped.await.unwrap();
            })
            .await
            .expect("connected measurement children shut down together");
        });
    }

    #[test]
    fn p27_task_shutdown_preserves_ready_transport_and_pipeline_failures() {
        run(async {
            let mut tasks = Tasks::default();
            tasks.spawn(async { Err(io::Error::other("early transport failure")) });
            tasks.spawn(async { Err(io::Error::other("early pipeline failure")) });
            let dropped = pending_child(&mut tasks);
            // This current-thread runtime has not polled any child yet. Both
            // failures are ready when the stop signal becomes ready as well.
            let failure = tasks.finish().await.unwrap_err().to_string();
            assert!(failure.contains("early transport failure"));
            assert!(failure.contains("early pipeline failure"));
            dropped.await.unwrap();
        });
    }

    #[test]
    fn p27_task_shutdown_reports_ack_loss_before_explicit_stop() {
        run(async {
            let mut tasks = Tasks::default();
            let (ack_tx, mut ack_rx) = tokio::sync::mpsc::channel::<()>(1);
            drop(ack_tx);
            tasks.spawn(async move {
                ack_rx
                    .recv()
                    .await
                    .ok_or_else(|| io::Error::other("wire ACK stream closed"))?;
                Ok(())
            });
            let failure = tasks.finish().await.unwrap_err().to_string();
            assert!(failure.contains("wire ACK stream closed"));
        });
    }

    #[test]
    fn p27_task_shutdown_rejects_early_success() {
        run(async {
            let mut tasks = Tasks::default();
            tasks.spawn(async { Ok(()) });
            let failure = tasks.finish().await.unwrap_err().to_string();
            assert!(failure.contains("returned: Ok(())"));
        });
    }

    #[test]
    fn p27_task_shutdown_preserves_panic_and_joins_remaining_children() {
        run(async {
            let mut tasks = Tasks::default();
            tasks.spawn(async { panic!("early measurement panic") });
            let dropped = pending_child(&mut tasks);
            let failure = tasks.finish().await.unwrap_err().to_string();
            assert!(failure.contains("early measurement panic"));
            dropped.await.unwrap();
        });
    }

    #[test]
    fn p27_task_shutdown_drop_aborts_owned_children() {
        run(async {
            let mut tasks = Tasks::default();
            let dropped = pending_child(&mut tasks);
            drop(tasks);
            dropped.await.unwrap();
        });
    }

    #[test]
    fn p27_task_shutdown_cancelled_finish_keeps_child_ownership() {
        run(async {
            let mut tasks = Tasks::default();
            let dropped = pending_child(&mut tasks);
            let mut finish = Box::pin(tasks.finish());
            std::future::poll_fn(|cx| {
                assert!(finish.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(finish);
            dropped.await.unwrap();
        });
    }
}
