//! Hash + proven peer + app identity bind content; names never key the cache.
use std::collections::VecDeque;
use std::io;
use super::app_icon::{IconRef, IconReply};

const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct IconCacheKey {
    /// Full relation/principal/device identity from the authenticated connection.
    pub peer: String,
    pub icon: IconRef,
}

#[derive(Default)]
pub(crate) struct IconCache {
    entries: VecDeque<(IconCacheKey, Vec<u8>)>,
    bytes: usize,
}

impl IconCache {
    pub(crate) fn get(&self, key: &IconCacheKey) -> Option<&[u8]> {
        self.entries.iter().find(|(stored, _)| stored == key).map(|(_, bytes)| bytes.as_slice())
    }

    pub(crate) fn insert(&mut self, key: IconCacheKey, reply: &IconReply) -> io::Result<()> {
        if key.peer.is_empty() { return Err(super::app_icon::invalid("icon peer context is absent")); }
        let png = reply.bytes_for(&key.icon)?;
        // Fully validate before retaining bytes; invalid content cannot poison a known hash.
        super::app_icon::decode_png(&png)?;
        self.retain(|stored| stored.peer != key.peer || stored.icon.app_id != key.icon.app_id);
        while self.bytes.saturating_add(png.len()) > MAX_CACHE_BYTES {
            let Some((_, evicted)) = self.entries.pop_front() else { break; };
            self.bytes -= evicted.len();
        }
        self.bytes += png.len();
        self.entries.push_back((key, png));
        Ok(())
    }

    pub(crate) fn forget_peer(&mut self, peer: &str) {
        self.retain(|key| key.peer != peer);
    }

    fn retain(&mut self, keep: impl Fn(&IconCacheKey) -> bool) {
        self.entries.retain(|(key, _)| keep(key));
        self.bytes = self.entries.iter().map(|(_, value)| value.len()).sum();
    }
}
