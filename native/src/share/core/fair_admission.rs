//! Work-conserving device fairness: all idle capacity is available to one peer.
use std::{collections::{HashMap, VecDeque}, hash::Hash, io, sync::{Arc, Mutex}};
use tokio::sync::Notify;

struct State<K> {
    sequence: u64,
    waiting: VecDeque<(u64, K)>,
    active: HashMap<K, usize>,
    used: usize,
}
pub(super) struct Pool<K> { state: Mutex<State<K>>, changed: Notify, capacity: usize }
pub(super) struct Ticket<K: Clone + Eq + Hash> { pool: Arc<Pool<K>>, id: u64 }
pub(super) struct Permit<K: Clone + Eq + Hash> { pool: Arc<Pool<K>>, key: K }

impl<K: Clone + Eq + Hash> Pool<K> {
    pub(super) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State { sequence: 0, waiting: VecDeque::new(), active: HashMap::new(), used: 0 }),
            changed: Notify::new(), capacity: capacity.max(1) })
    }
    pub(super) fn enqueue(self: &Arc<Self>, key: K) -> io::Result<Ticket<K>> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Share-Aufnahme gesperrt"))?;
        // Queued requests own bounded control frames. Two pool capacities
        // per device and eight capacities globally bound those retained frames;
        // an overflowing device cannot fill the queue of another identity.
        if state.waiting.len() >= self.capacity.saturating_mul(8)
            || state.waiting.iter().filter(|(_, peer)| *peer == key).count() >= self.capacity.saturating_mul(2) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "Share-Aufnahme-Warteschlange ist belegt"));
        }
        state.sequence = state.sequence.checked_add(1).ok_or_else(|| io::Error::other("Share-Aufnahme-ID erschöpft"))?;
        let id = state.sequence;
        state.waiting.push_back((id, key));
        drop(state); self.changed.notify_waiters();
        Ok(Ticket { pool: self.clone(), id })
    }
}

impl<K: Clone + Eq + Hash> Ticket<K> {
    pub(super) fn try_acquire(&self) -> io::Result<Option<Permit<K>>> {
        let mut state = self.pool.state.lock().map_err(|_| io::Error::other("Share-Aufnahme gesperrt"))?;
        if state.used >= self.pool.capacity { return Ok(None); }
        let eligible = state.waiting.iter().enumerate().min_by_key(|(index, (_, key))|
            (state.active.get(key).copied().unwrap_or(0), *index))
            .map(|(index, _)| index);
        if eligible.is_none_or(|index| state.waiting[index].0 != self.id) { return Ok(None); }
        let Some((_, key)) = eligible.and_then(|index| state.waiting.remove(index)) else { return Ok(None) };
        *state.active.entry(key.clone()).or_default() += 1;
        state.used += 1;
        drop(state); self.pool.changed.notify_waiters();
        Ok(Some(Permit { pool: self.pool.clone(), key }))
    }
    pub(super) async fn acquire(&self) -> io::Result<Permit<K>> {
        loop {
            let changed = self.pool.changed.notified(); tokio::pin!(changed); changed.as_mut().enable();
            if let Some(permit) = self.try_acquire()? { return Ok(permit); }
            changed.await;
        }
    }
}
impl<K: Clone + Eq + Hash> Drop for Ticket<K> {
    fn drop(&mut self) {
        let mut state = self.pool.state.lock().unwrap_or_else(|p| p.into_inner());
        state.waiting.retain(|(id, _)| *id != self.id);
        drop(state); self.pool.changed.notify_waiters();
    }
}
impl<K: Clone + Eq + Hash> Drop for Permit<K> {
    fn drop(&mut self) {
        let mut state = self.pool.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(active) = state.active.get_mut(&self.key) {
            *active = active.saturating_sub(1);
            if *active == 0 { state.active.remove(&self.key); }
        }
        state.used = state.used.saturating_sub(1);
        drop(state); self.pool.changed.notify_waiters();
    }
}

#[cfg(test)]
#[path = "fair_admission_task_tests.rs"]
mod task_tests;
