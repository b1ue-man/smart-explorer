use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Deterministic, non-repeating-looking bytes.
fn pattern(length: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// A source of the pattern that counts its `read` calls and can fail from a
/// position on.
struct Source {
    data: Vec<u8>,
    position: usize,
    reads: Arc<AtomicUsize>,
    fail_from: Option<usize>,
}

impl Source {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            position: 0,
            reads: Arc::default(),
            fail_from: None,
        }
    }
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail_from.is_some_and(|from| self.position >= from) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let length = buffer.len().min(self.data.len() - self.position);
        buffer[..length].copy_from_slice(&self.data[self.position..self.position + length]);
        self.position += length;
        Ok(length)
    }
}

/// A destination that is a function of each write.
struct Sink<F: FnMut(&[u8]) -> io::Result<()>>(F);

impl<F: FnMut(&[u8]) -> io::Result<()>> Write for Sink<F> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        (self.0)(buffer).map(|()| buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn run(
    source: &mut Source,
    write: impl FnMut(&[u8]) -> io::Result<()>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    (block, depth): (usize, usize),
) -> Result<Option<u64>, PipeError> {
    pipe_copy(source, &mut Sink(write), cancel, progress, block, depth)
}

#[test]
fn speed_task_pipe_copy_copies_all_bytes_in_order() {
    // Not a multiple of the block, several times the ring.
    for (length, block, depth) in [
        (5 * 1024 * 1024 + 123, 64 * 1024, 4),
        (1000, 7, 1),
        (9, 4, 3),
    ] {
        let data = pattern(length);
        let (mut written, mut reported) = (Vec::new(), 0u64);
        let done = run(
            &mut Source::new(data.clone()),
            |bytes| {
                written.extend_from_slice(bytes);
                Ok(())
            },
            &AtomicBool::new(false),
            &mut |bytes| reported += bytes,
            (block, depth),
        );
        assert_eq!(done.expect("copy"), Some(length as u64));
        assert!(written == data, "bytes differ for {length}/{block}/{depth}");
        assert_eq!(reported, length as u64, "progress sums to the length");
    }
}

#[test]
fn speed_task_pipe_copy_reader_error_is_reported_as_read_side() {
    let mut source = Source::new(pattern(300));
    source.fail_from = Some(200);
    let mut written = Vec::new();
    let error = run(
        &mut source,
        |bytes| {
            written.extend_from_slice(bytes);
            Ok(())
        },
        &AtomicBool::new(false),
        &mut |_| {},
        (100, 4),
    )
    .expect_err("the read fails");
    assert_eq!(error.side, PipeSide::Read);
    assert_eq!(error.error.kind(), io::ErrorKind::PermissionDenied);
    // What was read before the error has been written, in order.
    assert_eq!(written, pattern(300)[..200]);
}

#[test]
fn speed_task_pipe_copy_writer_error_stops_the_reader() {
    // A source far longer than the ring: the reader must not run to its end.
    let mut source = Source::new(vec![7; 1 << 20]);
    let reads = source.reads.clone();
    let mut writes = 0;
    let depth = 4;
    let error = run(
        &mut source,
        |_| {
            writes += 1;
            match writes {
                1..=2 => Ok(()),
                _ => Err(io::Error::from(io::ErrorKind::StorageFull)),
            }
        },
        &AtomicBool::new(false),
        &mut |_| {},
        (1024, depth),
    )
    .expect_err("the write fails");
    assert_eq!(error.side, PipeSide::Write);
    assert_eq!(error.error.kind(), io::ErrorKind::StorageFull);
    // Two blocks were written, the ring held at most `depth` more.
    let reads = reads.load(Ordering::SeqCst);
    assert!(reads <= 2 + depth + 1, "reader ran on: {reads}");
}

#[test]
fn speed_task_pipe_copy_cancel_returns_none() {
    // Set before the call: nothing is copied.
    let mut written = Vec::new();
    let result = run(
        &mut Source::new(pattern(4096)),
        |bytes| {
            written.extend_from_slice(bytes);
            Ok(())
        },
        &AtomicBool::new(true),
        &mut |_| {},
        (512, 4),
    );
    assert!(matches!(result, Ok(None)) && written.is_empty());
    // Set between blocks: the copy stops there, long before the end.
    let cancel = AtomicBool::new(false);
    let (mut written, mut seen) = (0usize, 0u64);
    let result = run(
        &mut Source::new(pattern(1 << 20)),
        |bytes| {
            written += bytes.len();
            Ok(())
        },
        &cancel,
        &mut |bytes| {
            seen += bytes;
            if seen >= 4096 {
                cancel.store(true, Ordering::Release);
            }
        },
        (1024, 2),
    );
    assert!(matches!(result, Ok(None)));
    assert!((4096..1 << 20).contains(&written), "stopped: {written}");
}

#[test]
fn speed_task_pipe_copy_reader_runs_ahead_up_to_depth() {
    let (depth, block) = (3, 100);
    let mut source = Source::new(pattern(block * 20));
    let reads = source.reads.clone();
    let release = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (reads, release) = (reads.clone(), release.clone());
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while reads.load(Ordering::SeqCst) < depth && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            // Give a reader that would run past the ring time to do so.
            std::thread::sleep(Duration::from_millis(150));
            let ahead = reads.load(Ordering::SeqCst);
            release.store(true, Ordering::SeqCst);
            ahead
        })
    };
    // The writer is stuck in its first write until the watcher has looked.
    let waiting = release.clone();
    let result = run(
        &mut source,
        move |_| {
            while !waiting.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        },
        &AtomicBool::new(false),
        &mut |_| {},
        (block, depth),
    );
    assert_eq!(result.expect("copy"), Some(2000));
    let ahead = watcher.join().expect("watcher");
    assert_eq!(ahead, depth, "the reader fills the ring and no more");
}

#[test]
fn speed_task_pipe_copy_empty_input() {
    let mut reported = 0u64;
    let result = run(
        &mut Source::new(Vec::new()),
        |_| panic!("nothing to write"),
        &AtomicBool::new(false),
        &mut |bytes| reported += bytes,
        (1024, 4),
    );
    assert_eq!(result.expect("copy"), Some(0));
    assert_eq!(reported, 0);
}
