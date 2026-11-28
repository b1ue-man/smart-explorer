//! Modules of the content search batch (plan docs/plaene/2026-11-27-dateien-
//! suche-ansicht, section B: the Share host searches the files it holds,
//! within the export's rights; only the hits travel): `share/mod.rs`
//! declares this file once and imports its modules.
#[path = "os/shared/host_content_search.rs"]
pub(super) mod host_content_search;
#[path = "core/peer_content_search.rs"]
pub(super) mod peer_content_search;
