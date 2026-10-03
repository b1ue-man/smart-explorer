//! Fair CPU-work admission: one active job per authenticated transport device.
use std::{collections::{HashSet, VecDeque}, sync::{Arc, Mutex, OnceLock}};
use tokio::sync::Notify;
use super::session::{PeerPrincipal, PeerDeviceKey};

#[derive(Default)]
struct State {
    sequence: u64,
    waiting: VecDeque<(u64, PeerDeviceKey)>,
    active: HashSet<PeerDeviceKey>,
}

pub(super) struct Admission {
    state: Mutex<State>,
    changed: Notify,
    workers: usize,
}

pub(super) fn listing() -> Arc<Admission> {
    static LISTING: OnceLock<Arc<Admission>> = OnceLock::new();
    LISTING.get_or_init(|| Arc::new(Admission { state: Mutex::new(State::default()), changed: Notify::new(),
        workers: std::thread::available_parallelism().map_or(1, |cores| cores.get()).clamp(1, 2) })).clone()
}

pub(super) fn watch_setup() -> Arc<Admission> {
    static WATCH: OnceLock<Arc<Admission>> = OnceLock::new();
    WATCH.get_or_init(|| Arc::new(Admission { state: Mutex::new(State::default()), changed: Notify::new(),
        workers: std::thread::available_parallelism().map_or(1, |cores| cores.get()).clamp(1, 2) })).clone()
}

pub(super) fn host() -> Arc<Admission> {
    static HOST: OnceLock<Arc<Admission>> = OnceLock::new();
    HOST.get_or_init(|| Arc::new(Admission {
        state: Mutex::new(State::default()), changed: Notify::new(),
        workers: (std::thread::available_parallelism().map_or(2, |cores| cores.get())
            / crate::analytics::local_scan_threads()).clamp(1, 2),
    })).clone()
}

pub(super) struct Ticket { admission: Arc<Admission>, id: u64 }
pub(super) struct Permit { admission: Arc<Admission>, principal: PeerDeviceKey }

impl Admission {
    pub(super) fn enqueue(self: &Arc<Self>, principal: PeerPrincipal) -> Ticket {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.sequence = state.sequence.wrapping_add(1);
        let id = state.sequence;
        state.waiting.push_back((id, principal.device_identity()));
        drop(state);
        self.changed.notify_waiters();
        Ticket { admission: self.clone(), id }
    }
}

impl Ticket {
    pub(super) fn position(&self) -> u32 {
        let state = self.admission.state.lock().unwrap_or_else(|p| p.into_inner());
        state.waiting.iter().position(|(id, _)| *id == self.id)
            .map_or(0, |index| u32::try_from(index + 1).unwrap_or(u32::MAX))
    }

    pub(super) async fn acquire(&self) -> Permit {
        loop {
            let changed = self.admission.changed.notified();
            tokio::pin!(changed);
            // Register before checking the state: a permit released between
            // checking and await must never strand this waiter.
            changed.as_mut().enable();
            {
                let mut state = self.admission.state.lock().unwrap_or_else(|p| p.into_inner());
                let eligible = state.waiting.iter().position(|(_, peer)| !state.active.contains(peer));
                if state.active.len() < self.admission.workers
                    && eligible.is_some_and(|index| state.waiting[index].0 == self.id)
                {
                    if let Some((_, principal)) = eligible.and_then(|index| state.waiting.remove(index)) {
                        state.active.insert(principal.clone());
                        drop(state);
                        self.admission.changed.notify_waiters();
                        return Permit { admission: self.admission.clone(), principal };
                    }
                }
            }
            changed.await;
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut state = self.admission.state.lock().unwrap_or_else(|p| p.into_inner());
        state.waiting.retain(|(id, _)| *id != self.id);
        drop(state);
        self.admission.changed.notify_waiters();
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.admission.state.lock().unwrap_or_else(|p| p.into_inner()).active.remove(&self.principal);
        self.admission.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_admission_skips_waiting_alias_of_active_device() {
        let runtime=tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        runtime.block_on(async {
            let admission=Arc::new(Admission { state:Mutex::new(State::default()),changed:Notify::new(),workers:2 });
            let direct=PeerPrincipal::new("direct","a","device","key-a","node-a");
            let room=PeerPrincipal::new("room","room","different-alias","key-a","node-a");
            let other=PeerPrincipal::new("direct","b","other","key-b","node-b");
            let first=admission.enqueue(direct); let active=first.acquire().await;
            let queued=admission.enqueue(room); let fair=admission.enqueue(other);
            let independent=tokio::time::timeout(std::time::Duration::from_secs(1),fair.acquire()).await.unwrap();
            assert_eq!(queued.position(),1,"same device keeps one queue entry across relationships");
            assert_eq!(admission.state.lock().unwrap().active.len(),2);
            drop(independent); drop(active);
            let next=tokio::time::timeout(std::time::Duration::from_secs(1),queued.acquire()).await.unwrap();
            drop(next); assert!(admission.state.lock().unwrap().active.is_empty());
        });
    }
}
