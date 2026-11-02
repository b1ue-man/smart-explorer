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
}
