//! The `Backend` methods that search next to the data: the name search, the
//! signature walk and the content search ("Suchen in Dateien", plan
//! docs/plaene/2026-11-27-dateien-suche-ansicht section B). `core.rs` expands
//! `server_search_methods!` inside the trait, so they are ordinary trait
//! methods with these defaults and doc comments; they live here so the trait
//! file stays under 500 lines. Paths resolve where the macro expands
//! (`vfs/core/core.rs`).

macro_rules! server_search_methods {
    () => {
        /// Recursive search on the server (the agent's `Search`), streaming matches?
        fn supports_search(&self) -> bool {
            false
        }

        /// Search below `root` on the server, streaming matches (paths relative to
        /// `root`) into `tx`. `Ok(false)` only for "unsupported" before any hit;
        /// every failure (transport, remote, cancel) is an error, never a fallback.
        fn search(
            &self,
            root: &str,
            spec: &crate::agent_proto::SearchSpec,
            tx: crossbeam_channel::Sender<SearchHit>,
            cancel: &std::sync::atomic::AtomicBool,
        ) -> VfsResult<bool> {
            super::backend_defaults::search(root, spec, tx, cancel)
        }

        /// Sync signatures (size, mtime, MD5 on demand) in one server-side walk
        /// (the agent's `WalkHashed`), without downloading files?
        fn supports_walk_hashed(&self) -> bool {
            false
        }

        /// Walk `root` on the server, streaming a `HashHit` per entry into `tx`
        /// (MD5 when `want_hash`). `Ok(false)` only for "unsupported" before any
        /// entry; later failures are errors, so partial walks never merge.
        fn walk_hashed(
            &self,
            root: &str,
            want_hash: bool,
            tx: crossbeam_channel::Sender<HashHit>,
            cancel: &std::sync::atomic::AtomicBool,
        ) -> VfsResult<bool> {
            super::backend_defaults::walk_hashed(root, want_hash, tx, cancel)
        }

        /// Content search next to the data (the SSH agent, a Share host): true
        /// when `content_search` may answer instead of reading the files.
        fn supports_content_search(&self) -> bool {
            false
        }

        /// Search the content of the files below `root` (a folder, or one file)
        /// next to the data, streaming events into `tx`. `Ok(None)` only before
        /// any event = not offered here; `vfs::content_search` then reads the
        /// files through this backend. Every later failure is an error.
        fn content_search(
            &self,
            root: &str,
            query: &crate::agent_proto::ContentQuery,
            tx: crossbeam_channel::Sender<crate::agent_proto::ContentEvent>,
            cancel: &std::sync::atomic::AtomicBool,
        ) -> VfsResult<Option<crate::agent_proto::ContentSummary>> {
            super::backend_defaults::content_search(root, query, tx, cancel)
        }
    };
}

pub(super) use server_search_methods;
