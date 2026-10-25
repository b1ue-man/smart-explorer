//! Topology never waits for capacity. One process-wide capacity worker owns all
//! potentially blocking storage probes, with one queued request and bounded replies.
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use crossbeam_channel::{Receiver, Sender};
use crate::app::drive_snapshot::DriveEntry;

pub(crate) type Invalidate = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct DriveMonitor {
    pub snapshots: Receiver<Result<Vec<DriveEntry>, String>>,
    refresh: Sender<()>,
    stop: Arc<AtomicBool>,
}

struct CapacityRequest {
    root: String,
    identity: Option<String>,
    generation: u64,
    reply: Sender<CapacityResult>,
}
struct CapacityResult { root: String, identity: Option<String>, generation: u64, free: u64, total: u64 }
static CAPACITY: OnceLock<Option<Sender<CapacityRequest>>> = OnceLock::new();

fn capacity_service() -> Option<Sender<CapacityRequest>> {
    CAPACITY.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::bounded::<CapacityRequest>(1);
        let spawned = std::thread::Builder::new().name("drive-capacity".into()).spawn(move || {
            while let Ok(request) = rx.recv() {
                let (free, total) = crate::app::platform_helpers::read_drive_capacity(&request.root);
                let CapacityRequest { root, identity, generation, reply } = request;
                let _ = reply.try_send(CapacityResult { root, identity, generation, free, total });
            }
        });
        match spawned {
            Ok(_) => Some(tx),
            Err(error) => { eprintln!("Laufwerkskapazität: {error}"); None }
        }
    }).clone()
}

// Coalesce to the latest complete projection without blocking on the GUI.
fn publish(tx: &Sender<Result<Vec<DriveEntry>, String>>,
    queued: &Receiver<Result<Vec<DriveEntry>, String>>,
    value: Result<Vec<DriveEntry>, String>, ctx: &eframe::egui::Context)
{
    if let Err(crossbeam_channel::TrySendError::Full(value)) = tx.try_send(value) {
        let _ = queued.try_recv();
        let _ = tx.try_send(value);
    }
    ctx.request_repaint();
}

impl DriveMonitor {
    pub fn start(ctx: eframe::egui::Context) -> Result<Self, String> {
        let (refresh, requests) = crossbeam_channel::bounded(1);
        let (results, snapshots) = crossbeam_channel::bounded(1);
        let queued = snapshots.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let signal = refresh.clone();
        let invalidate: Invalidate = Arc::new(move || { let _ = signal.try_send(()); });
        let events_stop = stop.clone();
        std::thread::Builder::new().name("drive-inventory".into()).spawn(move || {
            crate::app::platform_helpers::start_drive_events(invalidate, events_stop);
            let capacity = capacity_service();
            let (reply, capacities) = crossbeam_channel::bounded::<CapacityResult>(1);
            let mut current: Vec<DriveEntry> = Vec::new();
            let mut generations: BTreeMap<String, u64> = BTreeMap::new();
            let mut next_topology = Instant::now();
            let mut next_capacity = Instant::now();
            let mut cursor = 0usize;
            while !stopped.load(Ordering::Acquire) {
                if Instant::now() >= next_topology {
                    // The platform contract excludes capacity and network filesystem
                    // metadata here. A wedged network probe cannot stall this list.
                    match crate::app::platform_helpers::read_drive_inventory() {
                        Ok(mut entries) => {
                            for old in &current {
                                if !entries.iter().any(|entry| entry.root == old.root && entry.identity == old.identity) {
                                    let generation = generations.entry(old.root.clone()).or_default();
                                    *generation = generation.wrapping_add(1);
                                }
                            }
                            for entry in &mut entries {
                                generations.entry(entry.root.clone()).or_default();
                                if let Some(old) = current.iter().find(|old| old.root == entry.root && old.identity == entry.identity) {
                                    entry.free = old.free; entry.total = old.total;
                                }
                            }
                            current = entries;
                            publish(&results, &queued, Ok(current.clone()), &ctx);
                        }
                        Err(error) => publish(&results, &queued, Err(error), &ctx),
                    }
                    next_topology = Instant::now() + Duration::from_secs(3);
                }
                while let Ok(result) = capacities.try_recv() {
                    if generations.get(&result.root) != Some(&result.generation) { continue; }
                    if let Some(entry) = current.iter_mut().find(|entry|
                        entry.root == result.root && entry.identity == result.identity) {
                        if result.total > 0 && (entry.free != result.free || entry.total != result.total) {
                            entry.free = result.free; entry.total = result.total;
                            publish(&results, &queued, Ok(current.clone()), &ctx);
                        }
                    }
                }
                if !current.is_empty() && Instant::now() >= next_capacity {
                    if let Some(capacity) = &capacity {
                        let entry = &current[cursor % current.len()];
                        let request = CapacityRequest { root: entry.root.clone(), identity: entry.identity.clone(),
                            generation: generations.get(&entry.root).copied().unwrap_or_default(), reply: reply.clone() };
                        if capacity.try_send(request).is_ok() { cursor = cursor.wrapping_add(1); }
                    }
                    next_capacity = Instant::now() + Duration::from_millis(500);
                }
                match requests.recv_timeout(Duration::from_millis(100)) {
                    Ok(()) => {
                        // Bounded coalescing; storms cannot postpone topology forever.
                        std::thread::sleep(Duration::from_millis(120));
                        while requests.try_recv().is_ok() {}
                        next_topology = Instant::now();
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    Err(_) => {}
                }
            }
        }).map_err(|error| error.to_string())?;
        Ok(Self { snapshots, refresh, stop })
    }
    pub fn refresh(&self) { let _ = self.refresh.try_send(()); }
}

impl Drop for DriveMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.refresh();
    }
}
