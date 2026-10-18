//! Screen streaming and remote control between paired devices (strand S4).
//!
//! `core/` holds the platform-neutral parts: the wire formats of the control
//! stream and of the picture streams, the input model, codec negotiation,
//! the lossless tile codec, CPU pixel conversion, the synthetic test source,
//! rate control, pacing, metrics, session states and window lists (SCR-M1),
//! and the session itself (SCR-M2): host interfaces, host server, pipeline,
//! picture streams, viewer client, registry, rights and the measurement of
//! `se screen bench`. `os/` holds the platform side: the test-picture
//! backend and the indicator window, later capture, codecs and input.

pub(crate) mod core;
pub(crate) mod os;

/// Whether this process can show its screen to others (catalog rows, C7).
// Staged: the callers of `capability_catalog` pass `true` until their
// packages pass this (`share_exec_programs_ui.rs`, `grants_access.rs`,
// `mobile/os/shared/domains/share_access.rs`).
#[allow(dead_code)]
pub(crate) fn graphical_session() -> bool {
    os::synthetic_backend::graphical_session()
}
