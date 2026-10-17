//! Platform-neutral parts of the screen protocol (SCR-M1): no `cfg`, no OS
//! crates, no FFI. Each module owns one format or algorithm:
//! `input_model` (input events, HID key table, `INPUT` batch, release of held
//! inputs), `metrics` (latency, rates, clock offset, recovery, PSNR),
//! `negotiate` (codec offers and their ranking), `pacing` (queue guard,
//! stale picture streams, frame clock), `pixel_convert` (frame geometry, CPU
//! conversion), `rate_control` (target bit rate, quality ladder),
//! `session_state` (states, end reasons, key-frame requests),
//! `synthetic` (test picture with barcode), `tiles_codec` (lossless tile
//! codec), `window_list` (sources, monitors, windows, apps), `wire` (tags,
//! limits, framing), `wire_binary` (picture streams), `wire_control`
//! (control frames after the handshake) and `wire_hello` (hellos and the
//! relation proof of S1).

pub(crate) mod input_model;
pub(crate) mod metrics;
pub(crate) mod negotiate;
pub(crate) mod pacing;
pub(crate) mod pixel_convert;
pub(crate) mod rate_control;
pub(crate) mod session_state;
pub(crate) mod synthetic;
pub(crate) mod tiles_codec;
pub(crate) mod window_list;
pub(crate) mod wire;
pub(crate) mod wire_binary;
pub(crate) mod wire_control;
pub(crate) mod wire_hello;
