//! Packet downloads hand their finished members to the caller's thread
//! (G3-01, 2026-10-28). The packet's network futures run in `block_on` of
//! the backend's runtime on a fetcher thread of their own; the engine's
//! `BatchSink` runs on the calling thread, outside every runtime. The sink
//! may therefore call this or any other blocking backend (the receipts of a
//! recoverable job stat the source, which is another `block_on`: inside the
//! first one tokio panics with "Cannot start a runtime from within a
//! runtime"), and while it writes and publishes locally the fetcher keeps
//! polling the wire. Shared by the SFTP and SMB packet downloads.
use std::io;
use std::sync::mpsc;

/// What a fetcher calls for each finished member, in delivery order: an
/// error means the caller stopped taking members (start nothing more).
pub(crate) type Forward<'a, T> = dyn FnMut(usize, T) -> io::Result<()> + 'a;

/// Runs `fetch` on its own thread and calls `deliver` on this thread for
/// every member `fetch` forwards, in the order forwarded. A `deliver` error
/// closes the channel: the next `forward` of the fetcher fails, so it starts
/// nothing new and returns once its running members closed their handles;
/// the `deliver` error is returned. Without a `deliver` error, the result of
/// `fetch` is returned. A fetcher thread that cannot start refuses the packet
/// as `Unsupported` before anything is sent (the engine then moves the files
/// one by one).
pub(crate) fn hand_off<T, F>(
    thread_name: &str,
    fetch: F,
    mut deliver: impl FnMut(usize, T) -> io::Result<()>,
) -> io::Result<()>
where
    T: Send,
    F: FnOnce(&mut Forward<'_, T>) -> io::Result<()> + Send,
{
    let (sender, receiver) = mpsc::channel::<(usize, T)>();
    std::thread::scope(|scope| {
        let fetcher = std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn_scoped(scope, move || {
                let mut forward = |index: usize, member: T| {
                    sender.send((index, member)).map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Paket-Empfänger nimmt keine Dateien mehr an",
                        )
                    })
                };
                fetch(&mut forward)
            })
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("Paket-Download ohne eigenen Faden nicht möglich: {error}"),
                )
            })?;
        let mut failure = None;
        for (index, member) in receiver.iter() {
            if let Err(error) = deliver(index, member) {
                failure = Some(error);
                break;
            }
        }
        // Closing the channel tells the fetcher to start nothing more.
        drop(receiver);
        let fetched = fetcher
            .join()
            .map_err(|_| io::Error::other("Paket-Download-Faden wurde unerwartet beendet"))?;
        match failure {
            Some(error) => Err(error),
            None => fetched,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::hand_off;
    use std::io;

    /// The members arrive in order on the calling thread, and a runtime's
    /// `block_on` inside `deliver` works (the defect G3-01 was exactly that).
    #[test]
    fn batch_handoff_task_delivers_outside_the_fetchers_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let caller = std::thread::current().id();
        let mut seen = Vec::new();
        let result = hand_off(
            "handoff-test",
            |forward| {
                runtime.block_on(async {
                    for index in 0..4usize {
                        forward(index, index * 10)?;
                    }
                    Ok::<(), io::Error>(())
                })
            },
            |index, value: usize| {
                assert_eq!(std::thread::current().id(), caller);
                // A nested runtime call as the receipts' stat makes it.
                let inner = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .map_err(io::Error::other)?;
                inner.block_on(async { seen.push((index, value)) });
                Ok(())
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(seen, vec![(0, 0), (1, 10), (2, 20), (3, 30)]);
    }

    /// A `deliver` error stops the fetcher at its next member and is the
    /// result; nothing after it is delivered.
    #[test]
    fn batch_handoff_task_deliver_error_stops_the_fetcher() {
        let mut delivered = Vec::new();
        let mut attempted = 0usize;
        let result = hand_off(
            "handoff-test",
            |forward| {
                for index in 0..100usize {
                    attempted = index + 1;
                    if forward(index, ()).is_err() {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
                    }
                    // Let the receiver see the member before the next one.
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Ok(())
            },
            |index, ()| {
                delivered.push(index);
                if index == 2 {
                    Err(io::Error::new(io::ErrorKind::StorageFull, "full"))
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(
            result.map_err(|error| error.kind()),
            Err(io::ErrorKind::StorageFull)
        );
        assert_eq!(delivered, vec![0, 1, 2]);
        assert!(attempted < 100, "the fetcher stopped early ({attempted})");
    }
}
