//! Platform-neutral parts of the screen protocol: no `cfg`, no OS crates, no
//! FFI. SCR-M1 brought the formats and algorithms: `input_model` (input
//! events, HID key table, `INPUT` batch, release of held inputs), `metrics`
//! (latency, rates, clock offset, recovery, PSNR), `negotiate` (codec offers
//! and their ranking), `pacing` (queue guard, stale picture streams, frame
//! clock), `pixel_convert` (frame geometry, CPU conversion), `rate_control`
//! (target bit rate, quality ladder), `session_state` (states, end reasons,
//! key-frame requests), `synthetic` (test picture with barcode),
//! `tiles_codec` (lossless tile codec), `window_list` (sources, monitors,
//! windows, apps), `wire` (tags, limits, framing), `wire_binary` (picture
//! streams), `wire_control` (control frames after the handshake) and
//! `wire_hello` (hellos and the relation proof of S1).
//!
//! SCR-M2 brings the session: `host_backend` (what the host needs from its
//! platform, the consent broker of S1), `authorize` (rights, confirmation,
//! access log), `registry` (sessions, exclusive control, rights changes),
//! `host_server` and `host_pipeline` (the host side), `gop_stream` (picture
//! streams over QUIC), `viewer_client` (the viewer side), `bench` (`se
//! screen bench`) and `catalog_entries` (the screen rows of the S1 catalog).
//!
//! SCR-M3 brings H.264 and the host process: `codec_registry` (the encoders
//! and decoders of a process in their order of choice, `tiles` last),
//! `host_stage` (capture and encoding in the host process, the pipeline's
//! view of it) and `worker_wire` (the frames between daemon and host
//! process).
//!
//! Parts of SCR-M1 that SCR-M2 and SCR-M3 do not use yet keep
//! `allow(dead_code)` on their module; each such declaration names them.

pub(crate) mod authorize;
pub(crate) mod bench;
pub(crate) mod catalog_entries;
pub(crate) mod codec_registry;
pub(crate) mod gop_stream;
pub(crate) mod host_backend;
pub(crate) mod host_pipeline;
pub(crate) mod host_server;
pub(crate) mod host_stage;
// Not yet used by SCR-M2: `TOOLBAR_KEYS`, `key_usage`, `key_name`,
// `pointer_to_pixel`, `pixel_to_pointer` and `PressedInputs::is_empty`
// (input of SCR-M6 and the viewer of SCR-M7).
#[allow(dead_code)]
pub(crate) mod input_model;
// Not yet used by SCR-M2: `LatencySamples::count`.
#[allow(dead_code)]
pub(crate) mod metrics;
pub(crate) mod negotiate;
// Not yet used: `Pacer::frames_in_flight` (no field of `STATS` carries it;
// the viewer statistics of SCR-M7 are its consumer).
#[allow(dead_code)]
pub(crate) mod pacing;
// Used only on Windows so far: `Nv12View` and `nv12_to_bgra` (the Media
// Foundation decoder of SCR-M3); the desktop viewer of SCR-M7 uses them on
// every desktop.
#[allow(dead_code)]
pub(crate) mod pixel_convert;
// Not yet used by SCR-M2: `RateController::receive_rate_bps`.
#[allow(dead_code)]
pub(crate) mod rate_control;
pub(crate) mod registry;
// Not yet used by SCR-M2: `SessionTracker`, `SessionState::can_move_to`,
// `SessionState::is_terminal` and `KeyframeRequester::is_open`.
#[allow(dead_code)]
pub(crate) mod session_state;
pub(crate) mod synthetic;
pub(crate) mod tiles_codec;
pub(crate) mod viewer_client;
pub(crate) mod window_list;
pub(crate) mod wire;
pub(crate) mod wire_binary;
pub(crate) mod wire_control;
pub(crate) mod wire_hello;
pub(crate) mod worker_wire;
