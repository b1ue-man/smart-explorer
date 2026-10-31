//! A scripted SMB wire for the packet tests (batch_tests.rs): a file system
//! in memory and a virtual clock. Every request takes `RTT` units and any
//! number of them overlap; the executor advances the clock only when nothing
//! else can run, so the clock reads the longest chain of dependent round
//! trips.
use super::batch::{get_all, put_all, FileState, ReadResult, SmbWire, WireError, WireFuture};
use crate::vfs::{BatchGet, BatchPut, BatchPutOutcome, BatchSink};
use smb2::types::FileId;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::io::{self, Cursor};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

pub(super) const RTT: u64 = 10;

pub(super) struct Clock {
    pub(super) now: Cell<u64>,
    pub(super) timers: RefCell<Vec<(u64, Waker)>>,
}

pub(super) struct Sleep<'a> {
    clock: &'a Clock,
    ticks: u64,
    until: Option<u64>,
}

impl Future for Sleep<'_> {
    type Output = ();
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let (clock, ticks) = (self.clock, self.ticks);
        let until = *self.until.get_or_insert(clock.now.get() + ticks);
        if clock.now.get() >= until {
            return Poll::Ready(());
        }
        clock.timers.borrow_mut().push((until, cx.waker().clone()));
        Poll::Pending
    }
}

pub(super) struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Polls `future`; when it waits and nobody was woken, time jumps to the
/// next timer.
pub(super) fn run<F: Future>(clock: &Clock, future: F) -> F::Output {
    let flag = Arc::new(Flag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        flag.0.store(false, Ordering::SeqCst);
        if let Poll::Ready(done) = future.as_mut().poll(&mut cx) {
            return done;
        }
        if flag.0.load(Ordering::SeqCst) {
            continue;
        }
        let next = clock.timers.borrow().iter().map(|timer| timer.0).min();
        let next = next.expect("a waiting future needs a timer");
        clock.now.set(next);
        let mut timers = clock.timers.borrow_mut();
        let (due, rest): (Vec<_>, Vec<_>) = timers.drain(..).partition(|timer| timer.0 <= next);
        *timers = rest;
        drop(timers);
        due.into_iter().for_each(|timer| timer.1.wake());
    }
}

#[derive(Default)]
pub(super) struct Faults {
    pub(super) refuse_create: bool,
    pub(super) fail_write_more: bool,
    pub(super) lose_rename: bool,
    /// Extra latency of the first READ of a path.
    pub(super) slow_read: HashMap<String, u64>,
    pub(super) reparse: BTreeSet<String>,
    pub(super) links: BTreeSet<String>,
}

pub(super) struct Fake {
    pub(super) clock: Clock,
    pub(super) files: RefCell<BTreeMap<String, Vec<u8>>>,
    pub(super) handles: RefCell<HashMap<u64, String>>,
    pub(super) log: RefCell<Vec<String>>,
    pub(super) faults: RefCell<Faults>,
    pub(super) max_write: usize,
    pub(super) max_read: usize,
}

impl Fake {
    pub(super) fn new(max_write: usize, max_read: usize) -> Self {
        Self {
            clock: Clock {
                now: Cell::new(0),
                timers: RefCell::new(Vec::new()),
            },
            files: RefCell::new(BTreeMap::new()),
            handles: RefCell::new(HashMap::new()),
            log: RefCell::new(Vec::new()),
            faults: RefCell::new(Faults::default()),
            max_write,
            max_read,
        }
    }

    pub(super) fn wait(&self, extra: u64) -> Sleep<'_> {
        Sleep {
            clock: &self.clock,
            ticks: RTT + extra,
            until: None,
        }
    }

    pub(super) fn note(&self, line: String) {
        self.log.borrow_mut().push(line);
    }

    pub(super) fn logged(&self, prefix: &str) -> usize {
        let log = self.log.borrow();
        log.iter().filter(|line| line.starts_with(prefix)).count()
    }

    pub(super) fn refusal(kind: io::ErrorKind, created: bool) -> WireError {
        WireError {
            error: io::Error::new(kind, "scripted refusal"),
            answered: true,
            created,
        }
    }

    pub(super) fn handle(&self, path: &str, id: u64) -> FileState {
        self.handles.borrow_mut().insert(id, path.to_string());
        FileState {
            id: FileId {
                persistent: id,
                volatile: 0,
            },
            closed: false,
        }
    }
}

impl SmbWire for Fake {
    fn create_write_close_new<'a>(
        &'a self,
        stage: &'a str,
        data: &'a [u8],
        first_block: usize,
    ) -> WireFuture<'a, FileState> {
        Box::pin(async move {
            self.wait(0).await;
            if self.faults.borrow().refuse_create {
                return Err(Self::refusal(io::ErrorKind::PermissionDenied, false));
            }
            if self.files.borrow().contains_key(stage) {
                return Err(Self::refusal(io::ErrorKind::AlreadyExists, false));
            }
            self.files
                .borrow_mut()
                .insert(stage.to_string(), data[..first_block].to_vec());
            self.note(format!("create {stage} {first_block}"));
            let mut file = self.handle(stage, self.files.borrow().len() as u64);
            file.closed = first_block == data.len();
            Ok(file)
        })
    }

    fn write_more<'a>(
        &'a self,
        file: &'a FileState,
        offset: u64,
        data: &'a [u8],
    ) -> WireFuture<'a, ()> {
        Box::pin(async move {
            self.wait(0).await;
            self.note(format!("write {offset}"));
            if self.faults.borrow().fail_write_more {
                return Err(Self::refusal(io::ErrorKind::StorageFull, true));
            }
            let path = self.handles.borrow()[&file.id.persistent].clone();
            let mut files = self.files.borrow_mut();
            let content = files.entry(path).or_default();
            content.resize(offset as usize + data.len(), 0);
            content[offset as usize..].copy_from_slice(data);
            Ok(())
        })
    }

    fn close<'a>(&'a self, _file: &'a FileState) -> WireFuture<'a, ()> {
        Box::pin(async move {
            self.wait(0).await;
            self.note("close".to_string());
            Ok(())
        })
    }

    fn rename_no_replace<'a>(&'a self, from: &'a str, to: &'a str) -> WireFuture<'a, ()> {
        Box::pin(async move {
            self.wait(0).await;
            self.note(format!("rename {to}"));
            if self.faults.borrow().lose_rename {
                let error = io::Error::new(io::ErrorKind::TimedOut, "lost reply");
                return Err(WireError {
                    error,
                    answered: false,
                    created: false,
                });
            }
            let mut files = self.files.borrow_mut();
            if files.contains_key(to) {
                return Err(Self::refusal(io::ErrorKind::AlreadyExists, false));
            }
            let content = files.remove(from).unwrap_or_default();
            files.insert(to.to_string(), content);
            Ok(())
        })
    }

    fn remove<'a>(&'a self, path: &'a str) -> WireFuture<'a, ()> {
        Box::pin(async move {
            self.wait(0).await;
            self.note(format!("remove {path}"));
            self.files.borrow_mut().remove(path);
            Ok(())
        })
    }

    fn open_read_whole<'a>(&'a self, path: &'a str, expected: u64) -> WireFuture<'a, ReadResult> {
        Box::pin(async move {
            let extra = self
                .faults
                .borrow()
                .slow_read
                .get(path)
                .copied()
                .unwrap_or(0);
            self.wait(extra).await;
            self.note(format!("open {path}"));
            let content = self.files.borrow().get(path).cloned();
            let Some(content) = content else {
                return Err(Self::refusal(io::ErrorKind::NotFound, false));
            };
            let first = content
                .len()
                .min(expected.min(self.max_read as u64) as usize);
            let open = expected > self.max_read as u64;
            Ok(ReadResult {
                size: content.len() as u64,
                data: content[..first].to_vec(),
                file: open.then(|| self.handle(path, 1_000 + self.log.borrow().len() as u64)),
                reparse: self.faults.borrow().reparse.contains(path),
            })
        })
    }

    fn read_more<'a>(
        &'a self,
        file: &'a FileState,
        offset: u64,
        len: usize,
    ) -> WireFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.wait(0).await;
            self.note(format!("read_more {offset}"));
            let path = self.handles.borrow()[&file.id.persistent].clone();
            let files = self.files.borrow();
            let content = &files[&path];
            let end = (offset as usize + len).min(content.len());
            Ok(content[(offset as usize).min(end)..end].to_vec())
        })
    }

    fn read_followed<'a>(&'a self, path: &'a str) -> WireFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            self.wait(0).await;
            self.note(format!("followed {path}"));
            if self.faults.borrow().links.contains(path) {
                return Ok(None);
            }
            Ok(self.files.borrow().get(path).cloned())
        })
    }

    fn max_write(&self) -> usize {
        self.max_write
    }

    fn max_read(&self) -> usize {
        self.max_read
    }
}

pub(super) fn entries(count: usize, size: usize) -> (Vec<BatchPut>, Cursor<Vec<u8>>) {
    let mut bytes = Vec::new();
    let list = (0..count)
        .map(|index| {
            bytes.extend(vec![index as u8; size]);
            BatchPut {
                path: format!("/share/dir/file{index}.bin"),
                size: size as u64,
                mtime_ms: None,
            }
        })
        .collect();
    (list, Cursor::new(bytes))
}

pub(super) fn put(
    fake: &Fake,
    list: &[BatchPut],
    data: &mut Cursor<Vec<u8>>,
) -> Vec<BatchPutOutcome> {
    run(&fake.clock, put_all(fake, list, data))
}

pub(super) fn published(outcome: &BatchPutOutcome) -> &str {
    match outcome {
        BatchPutOutcome::Published(path) => path,
        BatchPutOutcome::Failed(error) => panic!("expected Published, got {error}"),
    }
}

pub(super) fn failure(outcome: BatchPutOutcome) -> io::Error {
    match outcome {
        BatchPutOutcome::Failed(error) => error,
        BatchPutOutcome::Published(path) => panic!("expected Failed, got {path}"),
    }
}

#[derive(Default)]
pub(super) struct Events(pub(super) Vec<String>);

impl BatchSink for Events {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        self.0.push(format!("begin {index} {size}"));
        Ok(())
    }
    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        self.0.push(format!("data {index} {}", bytes.len()));
        Ok(())
    }
    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        let outcome = result.map_or_else(|error| format!("{:?}", error.kind()), |()| "ok".into());
        self.0.push(format!("end {index} {outcome}"));
        Ok(())
    }
    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()> {
        self.0.push(format!("failed {index} {:?}", error.kind()));
        Ok(())
    }
}

pub(super) fn items(fake: &Fake, count: usize, size: usize) -> Vec<BatchGet> {
    (0..count)
        .map(|index| {
            let path = format!("/share/dir/f{index}");
            fake.files
                .borrow_mut()
                .insert(path.clone(), vec![index as u8; size]);
            BatchGet {
                path,
                id: None,
                size: size as u64,
            }
        })
        .collect()
}

pub(super) fn get(fake: &Fake, list: &[BatchGet]) -> Vec<String> {
    let mut sink = Events::default();
    run(&fake.clock, get_all(fake, list, &mut sink)).expect("the sink never fails");
    sink.0
}
