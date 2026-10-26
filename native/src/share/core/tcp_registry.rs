//! Active tunnels only. Handles remove entries on every exit path; neither
//! caller supplied IDs nor serialized status views grant remote authority.
use std::{collections::BTreeMap, io, sync::{Arc, Mutex, PoisonError}};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::watch;
use iroh::endpoint::Connection;
use super::tcp_forward::{TunnelId, TunnelView, MAX_TCP_TUNNELS};

#[derive(Default)]
pub(crate) struct TcpCounters {
    pub(crate) active: AtomicUsize,
    pub(crate) read: AtomicU64,
    pub(crate) written: AtomicU64,
}
struct Entry {
    view: TunnelView,
    connection: Connection,
    stop: watch::Sender<bool>,
    counters: Arc<TcpCounters>,
}
#[derive(Default)]
pub(crate) struct TcpRegistry { entries: Mutex<BTreeMap<TunnelId, Entry>> }
impl TcpRegistry {
    pub(crate) fn insert(self: &Arc<Self>, view: TunnelView, connection: Connection) -> io::Result<TcpRegistration> {
        let mut entries = self.entries.lock().map_err(|_| io::Error::other("TCP registry locked"))?;
        if entries.len() >= MAX_TCP_TUNNELS || entries.contains_key(&view.id) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "TCP tunnel admission exhausted or duplicate ID"));
        }
        let id = view.id.clone();
        let counters = Arc::new(TcpCounters::default());
        let (stop, cancelled) = watch::channel(false);
        entries.insert(id.clone(), Entry { view, connection, stop, counters: counters.clone() });
        Ok(TcpRegistration { registry: self.clone(), id, cancelled, counters })
    }
    pub(crate) fn list(&self) -> Vec<TunnelView> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.values().map(|entry| {
            let mut view = entry.view.clone();
            view.active_connections = entry.counters.active.load(Ordering::Relaxed);
            view.socket_read_bytes = entry.counters.read.load(Ordering::Relaxed);
            view.socket_written_bytes = entry.counters.written.load(Ordering::Relaxed);
            view.closing = *entry.stop.borrow();
            view
        }).collect()
    }
    pub(crate) fn status(&self, id: &TunnelId) -> Option<TunnelView> {
        self.list().into_iter().find(|view| &view.id == id)
    }
    pub(crate) fn close(&self, id: &TunnelId) -> bool {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = entries.get(id) else { return false; };
        entry.stop.send_replace(true);
        super::node_tcp::close_connection(&entry.connection);
        true
    }
    pub(crate) fn close_all(&self) {
        let ids: Vec<_> = self.list().into_iter().map(|view| view.id).collect();
        for id in ids { self.close(&id); }
    }
    pub(crate) fn record_error(&self, id: &TunnelId, error: &io::Error) {
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(entry) = entries.get_mut(id) {
                entry.view.last_error = Some(super::tcp_forward::shown_error(&error.to_string()));
            }
        }
    }
}
pub(crate) struct TcpRegistration {
    registry: Arc<TcpRegistry>,
    pub(crate) id: TunnelId,
    pub(crate) cancelled: watch::Receiver<bool>,
    pub(crate) counters: Arc<TcpCounters>,
}
impl TcpRegistration {
    pub(crate) fn registry(&self) -> Arc<TcpRegistry> { self.registry.clone() }
}
impl Drop for TcpRegistration {
    fn drop(&mut self) {
        if let Some(entry) = self.registry.entries.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id) {
            super::node_tcp::close_connection(&entry.connection);
        }
    }
}
