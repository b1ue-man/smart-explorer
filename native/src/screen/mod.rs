//! Screen streaming and remote control between paired devices (strand S4).
//!
//! `core/` holds the platform-neutral formats and algorithms of SCR-M1: the
//! wire formats of the control stream and of the picture streams, the input
//! model, codec negotiation, the lossless tile codec, CPU pixel conversion,
//! the synthetic test source, rate control, pacing, metrics, session states
//! and window lists. Capture, codecs and input of each platform follow under
//! `os/` from SCR-M2 on.

// Until SCR-M2 (host server, viewer client, IPC) calls into the core, only
// its tests use it; SCR-M2 removes this allowance.
#[allow(dead_code)]
pub(crate) mod core;
