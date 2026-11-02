//! A server's files in memory for segmented_stage_tests.rs: names point at
//! objects, handles are bound to objects, and a swap puts another object
//! under a name.
use super::segmented_stage::{Created, Facts, Identity, RangeHandle, RangeOpen, StageWire};
use smb2::msg::create::CreateAction;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// One file object of the server; names point at objects.
pub(super) struct Object {
    pub(super) content: Vec<u8>,
    pub(super) end_of_file: u64,
    pub(super) directory: bool,
    pub(super) reparse: bool,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) objects: HashMap<u64, Object>,
    pub(super) names: HashMap<String, u64>,
    pub(super) next: u64,
}

#[derive(Clone)]
pub(super) struct Store {
    pub(super) state: Arc<Mutex<State>>,
    pub(super) refused: Arc<AtomicBool>,
    /// Handles of the creator and of ranges that are open.
    pub(super) open: Arc<AtomicUsize>,
    /// Names created by an open (must stay 0: ranges only open).
    pub(super) created_by_open: Arc<AtomicUsize>,
    /// The server gives no identity at all.
    pub(super) no_identity: bool,
    /// The identity it gives is 0.
    pub(super) zero_identity: bool,
    /// The CREATE of a range answers with this action.
    pub(super) action: Arc<Mutex<CreateAction>>,
}

impl Store {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                next: 1,
                ..State::default()
            })),
            refused: Arc::new(AtomicBool::new(false)),
            open: Arc::new(AtomicUsize::new(0)),
            created_by_open: Arc::new(AtomicUsize::new(0)),
            no_identity: false,
            zero_identity: false,
            action: Arc::new(Mutex::new(CreateAction::FileOpened)),
        }
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Puts another object under `name` (a swap of the path).
    pub(super) fn swap(&self, name: &str, object: Object) -> u64 {
        let mut state = self.lock();
        let id = state.next;
        state.next += 1;
        state.objects.insert(id, object);
        state.names.insert(name.to_string(), id);
        id
    }

    pub(super) fn remove(&self, name: &str) {
        self.lock().names.remove(name);
    }

    pub(super) fn named(&self, name: &str) -> Option<Vec<u8>> {
        let state = self.lock();
        let id = state.names.get(name)?;
        state.objects.get(id).map(|object| object.content.clone())
    }

    pub(super) fn id_of(&self, name: &str) -> Option<u64> {
        self.lock().names.get(name).copied()
    }

    pub(super) fn object(&self, id: u64) -> Vec<u8> {
        self.lock()
            .objects
            .get(&id)
            .map(|object| object.content.clone())
            .unwrap_or_default()
    }
}

pub(super) fn foreign(len: usize) -> Object {
    Object {
        content: vec![0xEE; len],
        end_of_file: len as u64,
        directory: false,
        reparse: false,
    }
}

/// The creator's handle: counted while it is open.
pub(super) struct Hold(Arc<AtomicUsize>);

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) struct FakeHandle {
    pub(super) store: Store,
    pub(super) object: u64,
}

impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.store.open.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) struct FakeWriter {
    pub(super) handle: FakeHandle,
    pub(super) offset: u64,
    pub(super) expected: u64,
    pub(super) accepted: u64,
}

impl Write for FakeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.accepted + data.len() as u64 > self.expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "grown"));
        }
        let mut state = self.handle.store.lock();
        let object = state
            .objects
            .get_mut(&self.handle.object)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let at = (self.offset + self.accepted) as usize;
        object.content[at..at + data.len()].copy_from_slice(data);
        self.accepted += data.len() as u64;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.accepted != self.expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "short range"));
        }
        Ok(())
    }
}

impl RangeHandle for FakeHandle {
    fn into_writer(self: Box<Self>, offset: u64, len: u64) -> Box<dyn Write + Send> {
        Box::new(FakeWriter {
            handle: *self,
            offset,
            expected: len,
            accepted: 0,
        })
    }
}

impl StageWire for Store {
    fn create(&self, stage: &str, size: u64) -> io::Result<Option<Created>> {
        if self.lock().names.contains_key(stage) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        let id = self.swap(
            stage,
            Object {
                content: vec![0; size as usize],
                end_of_file: size,
                directory: false,
                reparse: false,
            },
        );
        self.open.fetch_add(1, Ordering::SeqCst);
        let identity = if self.no_identity {
            None
        } else if self.zero_identity {
            Some(Identity::Index {
                index: 0,
                created: 5,
            })
        } else {
            Some(Identity::Index {
                index: id,
                created: 5,
            })
        };
        Ok(Some(Created {
            identity,
            hold: Box::new(Hold(self.open.clone())),
        }))
    }

    fn open_range(&self, stage: &str) -> io::Result<RangeOpen> {
        // `FileOpen`: a missing name is an error, nothing is created.
        let Some(id) = self.id_of(stage) else {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        };
        let facts = {
            let state = self.lock();
            let object = &state.objects[&id];
            Facts {
                identity: Some(Identity::Index {
                    index: id,
                    created: 5,
                }),
                action: *self.action.lock().unwrap(),
                attributes: if object.reparse { 0x400 } else { 0 },
                end_of_file: object.end_of_file,
                directory: object.directory,
                delete_pending: false,
            }
        };
        self.open.fetch_add(1, Ordering::SeqCst);
        Ok(RangeOpen {
            facts,
            handle: Box::new(FakeHandle {
                store: self.clone(),
                object: id,
            }),
        })
    }

    fn discard(&self, stage: &str) -> io::Result<()> {
        self.remove(stage);
        Ok(())
    }

    fn refused(&self) -> bool {
        self.refused.load(Ordering::SeqCst)
    }

    fn refuse(&self) {
        self.refused.store(true, Ordering::SeqCst);
    }
}
