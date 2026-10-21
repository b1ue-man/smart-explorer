//! The scripted channel under the SFTP packet tests: an in-memory file
//! system and a virtual clock on which every request takes one round trip
//! and any number of requests overlap. No server, no real time.
use super::batch_ops::{BatchWire, WireError, WireFuture};
use futures_util::task::noop_waker;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// One request, from sending to answer.
pub(super) const RTT: u64 = 100;

/// Virtual time: requests wait for it, the driver moves it to the next due
/// request when everything is waiting.
#[derive(Default)]
pub(super) struct Sim {
    pub(super) now: Cell<u64>,
    pub(super) timers: RefCell<Vec<(u64, Waker)>>,
}

struct Tick {
    sim: Rc<Sim>,
    due: u64,
}

impl Future for Tick {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.sim.now.get() >= self.due {
            return Poll::Ready(());
        }
        self.sim
            .timers
            .borrow_mut()
            .push((self.due, cx.waker().clone()));
        Poll::Pending
    }
}

fn sleep(sim: &Rc<Sim>, ticks: u64) -> Tick {
    Tick {
        sim: sim.clone(),
        due: sim.now.get() + ticks,
    }
}

/// Polls `future` to its end, moving the clock whenever everything waits.
pub(super) fn drive<F: Future>(sim: &Rc<Sim>, future: F) -> F::Output {
    let mut future = Box::pin(future);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        let next = sim.timers.borrow().iter().map(|(due, _)| *due).min();
        let Some(next) = next else {
            panic!("nothing is waiting for time, the packet is stuck");
        };
        sim.now.set(next);
        let (due, later): (Vec<_>, Vec<_>) = sim
            .timers
            .borrow_mut()
            .drain(..)
            .partition(|(at, _)| *at <= next);
        *sim.timers.borrow_mut() = later;
        for (_, waker) in due {
            waker.wake();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Op {
    Open,
    Write,
    Close,
    Rename,
    Remove,
    Exists,
    OpenRead,
    Read,
}

/// What goes wrong with one request (the first one that matches).
#[derive(Clone, Copy)]
pub(super) enum Fault {
    /// The server answers with this failure; nothing happened.
    Status(io::ErrorKind),
    /// The request happened, the answer is lost.
    LostAfterEffect,
    /// Someone else owns the name: it exists, the answer is a plain failure.
    Taken,
}

pub(super) struct Node {
    pub(super) dir: bool,
    pub(super) data: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Fs {
    pub(super) nodes: BTreeMap<String, Node>,
    handles: HashMap<String, String>,
    next: u32,
    pub(super) open_now: usize,
    pub(super) open_max: usize,
    pub(super) requests: Vec<(Op, String)>,
    faults: Vec<(Op, String, Fault)>,
}

impl Fs {
    fn take_fault(&mut self, op: Op, path: &str) -> Option<Fault> {
        let at = self
            .faults
            .iter()
            .position(|(o, needle, _)| *o == op && path.contains(needle.as_str()))?;
        Some(self.faults.remove(at).2)
    }

    fn open(&mut self, path: &str) -> String {
        self.next += 1;
        let handle = format!("h{}", self.next);
        self.handles.insert(handle.clone(), path.to_string());
        self.open_now += 1;
        self.open_max = self.open_max.max(self.open_now);
        handle
    }
}

fn status(kind: io::ErrorKind, text: &str) -> io::Error {
    io::Error::new(kind, text.to_string())
}

pub(super) struct FakeWire {
    sim: Rc<Sim>,
    pub(super) fs: RefCell<Fs>,
    pub(super) open_handles: Option<u64>,
    pub(super) chunk: usize,
    /// Reads answer at most this many bytes.
    pub(super) cut: Option<usize>,
    /// Extra time per path.
    pub(super) delay: HashMap<String, u64>,
}

impl FakeWire {
    pub(super) fn new(sim: &Rc<Sim>) -> Self {
        Self {
            sim: sim.clone(),
            fs: RefCell::new(Fs::default()),
            open_handles: None,
            chunk: 4096,
            cut: None,
            delay: HashMap::new(),
        }
    }

    pub(super) fn file(self, path: &str, data: &[u8]) -> Self {
        let node = Node {
            dir: false,
            data: data.to_vec(),
        };
        self.fs.borrow_mut().nodes.insert(path.to_string(), node);
        self
    }

    pub(super) fn dir(self, path: &str) -> Self {
        let node = Node {
            dir: true,
            data: Vec::new(),
        };
        self.fs.borrow_mut().nodes.insert(path.to_string(), node);
        self
    }

    pub(super) fn fail(&self, op: Op, needle: &str, fault: Fault) {
        let entry = (op, needle.to_string(), fault);
        self.fs.borrow_mut().faults.push(entry);
    }

    fn path_of(&self, handle: &str) -> String {
        self.fs
            .borrow()
            .handles
            .get(handle)
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn count(&self, op: Op) -> usize {
        self.fs
            .borrow()
            .requests
            .iter()
            .filter(|(o, _)| *o == op)
            .count()
    }

    pub(super) fn content(&self, path: &str) -> Option<Vec<u8>> {
        self.fs
            .borrow()
            .nodes
            .get(path)
            .map(|node| node.data.clone())
    }

    pub(super) fn stages(&self) -> Vec<String> {
        let fs = self.fs.borrow();
        fs.nodes
            .keys()
            .filter(|path| path.contains(".se-upload-"))
            .cloned()
            .collect()
    }

    /// One request: it waits a round trip, then happens (or is refused).
    fn call<'a, T: 'a>(
        &'a self,
        op: Op,
        path: String,
        effect: impl FnOnce(&mut Fs) -> io::Result<T> + 'a,
    ) -> WireFuture<'a, T> {
        Box::pin(async move {
            let extra = self.delay.get(&path).copied().unwrap_or(0);
            sleep(&self.sim, RTT + extra).await;
            let mut fs = self.fs.borrow_mut();
            fs.requests.push((op, path.clone()));
            let fault = fs.take_fault(op, &path);
            let answered = |error| WireError {
                error,
                answered: true,
            };
            match fault {
                Some(Fault::Status(kind)) => return Err(answered(status(kind, "fixture"))),
                Some(Fault::Taken) => {
                    let node = Node {
                        dir: false,
                        data: Vec::new(),
                    };
                    fs.nodes.insert(path, node);
                    return Err(answered(status(io::ErrorKind::Other, "Failure")));
                }
                _ => {}
            }
            let value = effect(&mut fs).map_err(answered)?;
            match fault {
                Some(Fault::LostAfterEffect) => Err(WireError {
                    error: status(io::ErrorKind::TimedOut, "Timeout"),
                    answered: false,
                }),
                _ => Ok(value),
            }
        })
    }
}

impl BatchWire for FakeWire {
    fn open_new(&self, path: &str) -> WireFuture<'_, String> {
        let name = path.to_string();
        self.call(Op::Open, name.clone(), move |fs| {
            if fs.nodes.contains_key(&name) {
                return Err(status(io::ErrorKind::AlreadyExists, "exists"));
            }
            let node = Node {
                dir: false,
                data: Vec::new(),
            };
            fs.nodes.insert(name.clone(), node);
            Ok(fs.open(&name))
        })
    }

    fn write(&self, handle: &str, offset: u64, data: Vec<u8>) -> WireFuture<'_, ()> {
        let name = self.path_of(handle);
        self.call(Op::Write, name.clone(), move |fs| {
            let node = fs
                .nodes
                .get_mut(&name)
                .ok_or_else(|| status(io::ErrorKind::NotFound, "no file"))?;
            let (start, end) = (offset as usize, offset as usize + data.len());
            if node.data.len() < end {
                node.data.resize(end, 0);
            }
            node.data[start..end].copy_from_slice(&data);
            Ok(())
        })
    }

    fn close(&self, handle: &str) -> WireFuture<'_, ()> {
        let handle = handle.to_string();
        self.call(Op::Close, self.path_of(&handle), move |fs| {
            fs.handles.remove(&handle);
            fs.open_now = fs.open_now.saturating_sub(1);
            Ok(())
        })
    }

    fn rename(&self, from: &str, to: &str) -> WireFuture<'_, ()> {
        let (from, to) = (from.to_string(), to.to_string());
        self.call(Op::Rename, from.clone(), move |fs| {
            if fs.nodes.contains_key(&to) {
                return Err(status(io::ErrorKind::Other, "Failure"));
            }
            let node = fs
                .nodes
                .remove(&from)
                .ok_or_else(|| status(io::ErrorKind::NotFound, "no file"))?;
            fs.nodes.insert(to, node);
            Ok(())
        })
    }

    fn remove(&self, path: &str) -> WireFuture<'_, ()> {
        let name = path.to_string();
        self.call(Op::Remove, name.clone(), move |fs| {
            fs.nodes.remove(&name);
            Ok(())
        })
    }

    fn exists(&self, path: &str) -> WireFuture<'_, bool> {
        let name = path.to_string();
        self.call(Op::Exists, name.clone(), move |fs| {
            Ok(fs.nodes.contains_key(&name))
        })
    }

    fn open_read(&self, path: &str) -> WireFuture<'_, String> {
        let name = path.to_string();
        self.call(Op::OpenRead, name.clone(), move |fs| {
            if !fs.nodes.get(&name).is_some_and(|node| !node.dir) {
                return Err(status(io::ErrorKind::NotFound, "No such file"));
            }
            Ok(fs.open(&name))
        })
    }

    fn read(&self, handle: &str, offset: u64, len: u32) -> WireFuture<'_, Vec<u8>> {
        let name = self.path_of(handle);
        let cut = self.cut;
        self.call(Op::Read, name.clone(), move |fs| {
            let data = fs
                .nodes
                .get(&name)
                .map(|node| node.data.as_slice())
                .unwrap_or(&[]);
            let start = (offset as usize).min(data.len());
            let mut end = (start + len as usize).min(data.len());
            if let Some(cut) = cut {
                end = end.min(start + cut);
            }
            Ok(data[start..end].to_vec())
        })
    }

    fn write_chunk(&self, _handle_len: usize) -> usize {
        self.chunk
    }

    fn read_chunk(&self) -> u32 {
        self.chunk as u32
    }

    fn open_handles(&self) -> Option<u64> {
        self.open_handles
    }
}
