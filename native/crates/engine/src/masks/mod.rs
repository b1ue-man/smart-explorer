//! Mask lists (plan docs/plaene/2026-11-28-ersetzen-masken-shell, S2 and the
//! contract „Verträge zwischen den Paketen“ point 1).
//!
//! A mask list holds entries (literal, Rust `regex`, fuzzy with a tolerance),
//! protected paths in se target notation, an enabled and a locked flag. Every
//! output to an agent runs through [`MaskSet::mask`] (real value →
//! placeholder `⟦Label-N⟧`, a found variant `⟦Label-N.V⟧`), every input of the
//! agent through [`MaskSet::unmask`] (placeholder → real value). The
//! assignment placeholder ↔ value is kept per list and never reused, so a
//! placeholder means the same value in every process and session.
//!
//! - `core/`: data model, folding, fuzzy rule (two budgets, reference
//!   docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md §2.4), matching,
//!   placeholders, the masking set and the stream masker. Platform-free.
//! - `os/`: the sealed store under `<app data>/masks/` (ChaCha20-Poly1305,
//!   key in the se secret store), `state.json` for the agent hook, the
//!   integrity guard, the guarded paths (se app data, agent host
//!   configuration).
//!
//! Without an enabled entry the masking set is empty and leaves every text
//! byte-identical, so surfaces without mask lists behave as before.
// The CLI, the MCP layer and the GUI consume this API (packages P4a, P5, P6);
// helpers of the model that only they reach are unused inside this crate.
#![allow(dead_code)]

mod core;
mod os;

pub use self::core::mask_set::{MaskSet, MaskedText};
pub use self::core::model::{
    Actor, EntryId, EntryKind, EntrySummary, EntryUpdate, EntryWithValue, ListId, ListSummary,
    LockdownState, MaskError, NewEntry, Tampered, Tolerance, UnmaskError, DEFAULT_LABEL,
};
pub use self::core::placeholder::{is_placeholder_text, PLACEHOLDER_CLOSE, PLACEHOLDER_OPEN};
pub use self::core::scope::{Scope, ScopeKind};
pub use self::core::stream::{StreamMasker, STREAM_IDLE_RELEASE};
pub use self::os::guard::IntegrityGuard;
pub use self::os::guarded::{guarded_overlap, guarded_paths, refers_to_guarded};
pub use self::os::store::{lockdown_state, MaskStore};
