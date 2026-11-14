//! Commands to the mDNS daemon wait for room in its command queue.
//!
//! `mdns-sd` hands every command (register, unregister, shutdown) to the
//! daemon thread through a bounded queue of 100 entries and answers
//! `Error::Again` ("try again") while it is full (`ServiceDaemon::send_cmd`,
//! `bounded(100)`). An announcement with many retiring lookup ids after a code
//! renewal registers two records per generation in one burst (63 generations:
//! 128 records), so the burst can outrun the daemon thread; withdrawing it
//! unregisters as many in one burst. Giving up on the first `Again` would lose
//! announcements or leave records announced that nothing removes later.
use std::time::{Duration, Instant};

/// Pause before the command is offered again; the daemon thread takes a
/// queued command in far less time.
const PAUSE: Duration = Duration::from_millis(5);
/// How long one command waits for room before the error is reported.
const LIMIT: Duration = Duration::from_secs(10);

/// Runs `send` again while the daemon's queue is full; every other result,
/// and `Again` after [`LIMIT`], is returned as it is.
pub(super) fn send_when_room<T>(
    mut send: impl FnMut() -> Result<T, mdns_sd::Error>,
) -> Result<T, mdns_sd::Error> {
    let started = Instant::now();
    loop {
        match send() {
            Err(mdns_sd::Error::Again) if started.elapsed() < LIMIT => std::thread::sleep(PAUSE),
            other => return other,
        }
    }
}
