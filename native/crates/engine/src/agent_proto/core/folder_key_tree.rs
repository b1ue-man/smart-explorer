//! The folders of one keyed walk (`folder_key.rs`): which run goes out when,
//! and which key. `Mode::Marks` holds runs back while a known key is
//! possible and answers a known subtree with one mark; `Mode::KeysOnly`
//! sends no run, only every folder's key (the first phase of a pair
//! comparison); `Mode::RunsOnly` sends each run as soon as it ends and no key
//! (the folders a client asked for by name).
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::{Mutex, MutexGuard};

use super::super::key_tally::KeyTally;
use super::{folder_key, known_prefix, run_digest, FolderKey, KnownKey, RunKeys, BUFFER_BYTES};

/// Where a `KeyTree` sends what is decided.
pub trait KeyEmit<I> {
    /// The complete run of the folder `rel`, in the order it was produced
    /// (sent after a run mark naming `rel`, so the client knows where every
    /// run begins, an empty one included).
    fn run(&mut self, rel: &str, items: Vec<I>) -> io::Result<()>;
    /// `same = false`: the end mark of a folder sent with everything below
    /// it; `same = true`: the folder's subtree is the one the client knows
    /// under `key` and is not sent. `tally` (`Mode::KeysOnly` only): what the
    /// subtree holds.
    fn key(
        &mut self,
        rel: &str,
        key: &FolderKey,
        same: bool,
        tally: Option<KeyTally>,
    ) -> io::Result<()>;
}

struct Node<I> {
    pending: usize,
    /// Subfolders it lists to be walked whose run has not come yet: a run
    /// is taken only once, and only for such a subfolder.
    waiting: HashSet<String>,
    run: [u8; 32],
    children: Vec<(String, FolderKey)>,
    items: Option<Vec<I>>,
    bytes: usize,
    same_children: Vec<(String, FolderKey)>,
    /// The run's own counts, then the subtree's once every child is added.
    tally: KeyTally,
}

struct State<I> {
    nodes: HashMap<String, Node<I>>,
    /// Folders whose run was held back, oldest first (spill order); an
    /// entry whose run went out since is skipped when it comes up.
    held: VecDeque<String>,
    buffered: usize,
    /// The root's run came (a second one is refused).
    root_started: bool,
    root_done: bool,
}

/// What a keyed walk sends (see the module head).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Marks,
    KeysOnly,
    RunsOnly,
}

/// The folders of one keyed walk (see the module head).
pub struct KeyTree<I> {
    root: String,
    known: HashSet<KnownKey>,
    mode: Mode,
    state: Mutex<State<I>>,
}

impl<I> KeyTree<I> {
    /// A walk whose first folder is `root` (its rel, usually empty).
    pub fn new(root: &str, known: HashSet<KnownKey>) -> Self {
        Self::with_mode(root, known, Mode::Marks)
    }

    /// A walk of `mode` (`known` counts only for `Mode::Marks`).
    pub fn with_mode(root: &str, known: HashSet<KnownKey>, mode: Mode) -> Self {
        Self {
            root: root.to_string(),
            known,
            mode,
            state: Mutex::new(State {
                nodes: HashMap::new(),
                held: VecDeque::new(),
                buffered: 0,
                root_started: false,
                root_done: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<I>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether the root folder and everything below it is decided (a
    /// `Mode::RunsOnly` walk has no root and is complete when it ends).
    pub fn complete(&self) -> bool {
        self.mode == Mode::RunsOnly || self.lock().root_done
    }

    fn parent_of<'r>(&self, rel: &'r str) -> Option<&'r str> {
        if rel == self.root {
            return None;
        }
        Some(rel.rsplit_once('/').map_or("", |(parent, _)| parent))
    }

    /// One folder's run ended (`ordered_walk` calls this before any folder
    /// below it starts). Emission happens under the tree's lock, so runs and
    /// marks reach the stream in the order decided here.
    pub fn finish_run(&self, run: RunKeys<I>, emit: &mut dyn KeyEmit<I>) -> io::Result<()> {
        let RunKeys {
            rel,
            parts,
            items,
            bytes,
            tally,
            dirs,
            unopened,
        } = run;
        if self.mode == Mode::RunsOnly {
            let _order = self.lock();
            return emit.run(&rel, items);
        }
        let waiting: HashSet<String> = dirs
            .into_iter()
            .filter(|name| !unopened.contains(name))
            .collect();
        let walked = waiting.len();
        let node = Node {
            pending: walked,
            waiting,
            run: run_digest(parts),
            children: Vec::new(),
            items: (self.mode == Mode::Marks).then_some(items),
            bytes,
            same_children: Vec::new(),
            tally,
        };
        let mut state = self.lock();
        // Every run but the root's belongs to a subfolder its parent listed
        // to be walked and that has not come before; a folder read twice or
        // one its parent reported as not opened would give a wrong key.
        match self.parent_of(&rel) {
            None if state.root_started => return Err(invalid("walk read one folder twice")),
            None => state.root_started = true,
            Some(parent) => {
                let listed = state
                    .nodes
                    .get_mut(parent)
                    .is_some_and(|parent| parent.waiting.remove(name_in(&rel)));
                if !listed {
                    return Err(invalid(
                        "walk read a folder twice or one its parent did not walk",
                    ));
                }
            }
        }
        if state.nodes.insert(rel.clone(), node).is_some() {
            return Err(invalid("walk read one folder twice"));
        }
        if self.mode == Mode::KeysOnly {
            // No run goes out; the key follows when the folder is complete.
        } else if self.known.is_empty() {
            self.emit_chain(&mut state, &rel, emit)?;
        } else {
            state.buffered = state.buffered.saturating_add(bytes);
            state.held.push_back(rel.clone());
            while state.buffered > BUFFER_BYTES {
                let Some(oldest) = state.held.pop_front() else {
                    break;
                };
                self.emit_chain(&mut state, &oldest, emit)?;
            }
        }
        if walked == 0 {
            self.completed(&mut state, rel, emit)?;
        }
        Ok(())
    }

    /// Sends the runs of `rel` and of every ancestor not sent yet, top down,
    /// each followed by the `same` marks of its known children.
    fn emit_chain(
        &self,
        state: &mut State<I>,
        rel: &str,
        emit: &mut dyn KeyEmit<I>,
    ) -> io::Result<()> {
        let mut chain = Vec::new();
        let mut current = Some(rel.to_string());
        while let Some(folder) = current {
            match state.nodes.get(&folder) {
                Some(node) if node.items.is_some() => {}
                _ => break,
            }
            current = self.parent_of(&folder).map(str::to_string);
            chain.push(folder);
        }
        for folder in chain.into_iter().rev() {
            let Some(node) = state.nodes.get_mut(&folder) else {
                continue;
            };
            let Some(items) = node.items.take() else {
                continue;
            };
            let bytes = node.bytes;
            let same = std::mem::take(&mut node.same_children);
            state.buffered = state.buffered.saturating_sub(bytes);
            emit.run(&folder, items)?;
            for (name, key) in same {
                emit.key(&child_rel(&folder, &name), &key, true, None)?;
            }
        }
        Ok(())
    }

    /// `rel` and everything below it are read: its key is final.
    fn completed(
        &self,
        state: &mut State<I>,
        rel: String,
        emit: &mut dyn KeyEmit<I>,
    ) -> io::Result<()> {
        let mut rel = rel;
        loop {
            let node = state
                .nodes
                .get_mut(&rel)
                .ok_or_else(|| invalid("walk completed an unknown folder"))?;
            let key = folder_key(&node.run, std::mem::take(&mut node.children));
            let tally = node.tally;
            let unsent = node.items.is_some();
            let same = unsent && self.known.contains(&known_prefix(&key));
            let parent = self.parent_of(&rel).map(str::to_string);
            if same {
                let node = state
                    .nodes
                    .remove(&rel)
                    .ok_or_else(|| invalid("walk lost a folder"))?;
                state.buffered = state.buffered.saturating_sub(node.bytes);
                let parent_waits = parent
                    .as_ref()
                    .and_then(|parent| state.nodes.get(parent))
                    .is_some_and(|parent| parent.items.is_some());
                if parent_waits {
                    let name = name_in(&rel).to_string();
                    if let Some(parent) = parent.as_ref().and_then(|p| state.nodes.get_mut(p)) {
                        parent.same_children.push((name, key));
                    }
                } else {
                    emit.key(&rel, &key, true, None)?;
                }
            } else {
                self.emit_chain(state, &rel, emit)?;
                emit.key(
                    &rel,
                    &key,
                    false,
                    (self.mode == Mode::KeysOnly).then_some(tally),
                )?;
                state.nodes.remove(&rel);
            }
            let Some(parent) = parent else {
                state.root_done = true;
                return Ok(());
            };
            let node = state
                .nodes
                .get_mut(&parent)
                .ok_or_else(|| invalid("walk completed a folder before its parent"))?;
            node.children.push((name_in(&rel).to_string(), key));
            node.tally.add_child(&tally);
            node.pending = node
                .pending
                .checked_sub(1)
                .ok_or_else(|| invalid("walk completed more subfolders than it listed"))?;
            if node.pending > 0 {
                return Ok(());
            }
            rel = parent;
        }
    }
}

fn child_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn name_in(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
