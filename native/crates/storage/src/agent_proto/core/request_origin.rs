//! The origin of one backend request (plan 2026-11-14, E4d). A process that
//! asks its background service for a backend over the IPC (a drive host,
//! the device tree, Android's documents provider, the desktop app, `se`)
//! does some of that work by itself: a cache refresh, a preload, the upload
//! of a closed file, a retry (`crate::vfs::BackgroundScope`). Nobody waits
//! for its answer, so the service must not count it as a person's use of a
//! Share device (metadata presence), although the handle serves a person.
//!
//! The mark travels with the request, not over a connection of its own: a
//! client sets bit 62 of the request id when the calling thread runs
//! background work and the server announced `origin-v1`; the server holds a
//! `BackgroundScope` while it answers that one request. Every frame of the
//! request (its reply, credit, cancel, data) keeps the id, so the mark needs
//! no field in any frame body and the body encoding stays byte for byte.
//!
//! Older peers: a server without the label never gets a marked id (the
//! client checks the label), and a server reads the id as an opaque number
//! anyway (`frame_io.rs`). A client without the mark counts from 1 and never
//! reaches the bit, so its requests keep the origin of the handle.
use crate::vfs::BackgroundScope;

/// A server that reads the background mark of a request announces this.
pub const LABEL_ORIGIN: &str = "origin-v1";

/// The bit of a request id that marks the request as background work. Far
/// above every counted id (a connection counts from 1, one per request).
pub const BACKGROUND_REQUEST: u64 = 1 << 62;

/// The id a client sends for its fresh request `id`: marked when the calling
/// thread runs background work and the server reads the mark.
pub fn request_id(id: u64, server_reads_origin: bool) -> u64 {
    if server_reads_origin && crate::vfs::in_background() {
        id | BACKGROUND_REQUEST
    } else {
        id
    }
}

/// Whether request `id` is background work of its client.
pub fn is_background_request(id: u64) -> bool {
    id & BACKGROUND_REQUEST != 0
}

/// The scope a server holds on the thread that answers request `id`, for
/// that request only; `None` keeps the origin of the served handle.
pub fn request_scope(id: u64) -> Option<BackgroundScope> {
    is_background_request(id).then(BackgroundScope::enter)
}

#[cfg(test)]
#[path = "request_origin_tests.rs"]
mod tests;
