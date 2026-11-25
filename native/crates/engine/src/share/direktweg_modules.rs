//! Modules of the direct-path batch (user report 2026-11-25: connections go
//! direct whenever possible, the address refresh runs at connection setup):
//! `share/mod.rs` declares this file once and imports its modules.
#[path = "core/peer_address_book.rs"]
pub(super) mod peer_address_book;
#[path = "core/peer_dial.rs"]
pub(super) mod peer_dial;
#[path = "core/route_retry_port.rs"]
pub(super) mod route_retry_port;
