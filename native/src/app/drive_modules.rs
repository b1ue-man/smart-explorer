//! Desktop-app files of strand S2 (drives) of plan 2026-10-17 „Rechte, se
//! Desktop, Tresore“. `app/mod.rs` includes this file once (R2); later S2
//! packages declare their app files here instead of in `app/mod.rs`.

// DRV-M3: „Einhängeort“ and „Beim Anmelden automatisch einbinden“ of the
// window „Remote als Laufwerk“.
#[path = "core/mount_ui_location.rs"]
pub(crate) mod mount_ui_location;

/// Platform fact for `mount_ui_location`: Windows drives sit at a drive
/// letter (strand S2 C1); every other desktop platform places a drive at a
/// folder (S2 C2, DRV-M4). Kept out of `core/` (AGENTS.md).
pub(crate) const DRIVES_USE_LETTERS: bool = cfg!(windows);
