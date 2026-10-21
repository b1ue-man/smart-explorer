//! MD5 of the bytes one sync copy moves, without taking its time from the read
//! and write loop. The first mebibyte is hashed by the calling thread (a
//! helper thread starts in 30 to 100 us while MD5 over 1 MiB takes about
//! 1.7 ms, so a helper pays off at the earliest from there); everything after
//! it goes to one helper thread of this copy. If the system refuses the
//! thread the calling thread keeps hashing, as before.
use std::io;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::JoinHandle;

/// Bytes hashed by the calling thread before a helper takes over.
pub(super) const INLINE_BYTES: usize = 1024 * 1024;
/// Blocks the helper may lag behind (256 KiB each in the sync copy loop).
const QUEUED_BLOCKS: usize = 4;

/// What the channel carries: the state after the first mebibyte, then data.
pub(super) enum Message {
    Start(Box<md5::Context>),
    Data(Vec<u8>),
}

/// Starts the helper that drains `Receiver` and returns the digest. A refusal
/// leaves the hasher inline.
pub(super) type Spawner = fn(Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>>;

pub(super) struct StreamHasher {
    /// The state while hashing inline; `None` once the helper owns it.
    context: Option<md5::Context>,
    inline_bytes: usize,
    sender: Option<SyncSender<Message>>,
    helper: Option<JoinHandle<[u8; 16]>>,
    /// The helper vanished (panicked); `finish` reports it.
    helper_lost: bool,
    /// The system refused the thread once; stay inline.
    refused: bool,
    spawner: Spawner,
}

impl StreamHasher {
    pub(super) fn new() -> Self {
        Self::with_spawner(spawn_helper)
    }

    pub(super) fn with_spawner(spawner: Spawner) -> Self {
        Self {
            context: Some(md5::Context::new()),
            inline_bytes: 0,
            sender: None,
            helper: None,
            helper_lost: false,
            refused: false,
            spawner,
        }
    }

    /// Whether a helper thread is running for this hash.
    #[cfg(test)]
    pub(super) fn uses_helper(&self) -> bool {
        self.helper.is_some()
    }

    /// Adds `bytes` in stream order.
    pub(super) fn update(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        if let Some(context) = self.context.as_mut() {
            let room = if self.refused {
                rest.len()
            } else {
                INLINE_BYTES.saturating_sub(self.inline_bytes)
            };
            let (now, later) = rest.split_at(room.min(rest.len()));
            context.consume(now);
            self.inline_bytes = self.inline_bytes.saturating_add(now.len());
            rest = later;
            if rest.is_empty() {
                return;
            }
            // More than the first mebibyte: only now a helper is worth it.
            self.start_helper();
            if self.context.is_some() {
                // Refused: the calling thread hashes the rest as well.
                if let Some(context) = self.context.as_mut() {
                    context.consume(rest);
                }
                return;
            }
        }
        self.send(rest);
    }

    fn start_helper(&mut self) {
        let (sender, receiver) = sync_channel(QUEUED_BLOCKS);
        let handle = match (self.spawner)(receiver) {
            Ok(handle) => handle,
            Err(_) => {
                self.refused = true;
                return;
            }
        };
        let Some(context) = self.context.take() else {
            return;
        };
        self.helper = Some(handle);
        if sender.send(Message::Start(Box::new(context))).is_err() {
            self.helper_lost = true;
        }
        self.sender = Some(sender);
    }

    fn send(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.helper_lost {
            return;
        }
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        if sender.send(Message::Data(bytes.to_vec())).is_err() {
            self.helper_lost = true;
        }
    }

    /// The digest of everything added; waits for the helper to drain.
    pub(super) fn finish(mut self) -> io::Result<[u8; 16]> {
        if let Some(context) = self.context.take() {
            return Ok(context.compute().0);
        }
        // Closing the channel ends the helper's loop.
        self.sender = None;
        let Some(handle) = self.helper.take() else {
            return Err(lost());
        };
        let digest = handle.join().map_err(|_| lost())?;
        if self.helper_lost {
            return Err(lost());
        }
        Ok(digest)
    }
}

fn lost() -> io::Error {
    io::Error::other("sync hash helper ended without a digest")
}

fn spawn_helper(receiver: Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>> {
    std::thread::Builder::new()
        .name("sync-md5".into())
        .spawn(move || run_helper(receiver))
}

fn run_helper(receiver: Receiver<Message>) -> [u8; 16] {
    crate::transfer::lower_thread_priority();
    let mut context = md5::Context::new();
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Start(started) => context = *started,
            Message::Data(bytes) => context.consume(&bytes),
        }
    }
    context.compute().0
}
