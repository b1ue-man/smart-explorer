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
//! Parts of SCR-M1 that SCR-M2 does not use yet keep `allow(dead_code)` on
//! their module; each such declaration names them.

pub(crate) mod authorize;
pub(crate) mod bench;
pub(crate) mod catalog_entries;
pub(crate) mod gop_stream;
pub(crate) mod host_backend;
pub(crate) mod host_pipeline;
pub(crate) mod host_server;
// Not yet used by SCR-M2: `TOOLBAR_KEYS`, `key_usage`, `key_name`,
// `pointer_to_pixel`, `pixel_to_pointer` and `PressedInputs::is_empty`
// (input of SCR-M6 and the viewer of SCR-M7).
#[allow(dead_code)]
pub(crate) mod input_model;
// Not yet used by SCR-M2: `LatencySamples::count`.
#[allow(dead_code)]
pub(crate) mod metrics;
// Not yet used by SCR-M2: `CodecOffer::h264` and `CodecOffer::with_max`
// (H.264 encoders and decoders of SCR-M3).
#[allow(dead_code)]
pub(crate) mod negotiate;
// Not yet used by SCR-M2: `RELAY_QUEUE_BYTES` and `Pacer::frames_in_flight`.
#[allow(dead_code)]
pub(crate) mod pacing;
// Not yet used by SCR-M2: `I420Image`, `Nv12View`, `YuvMatrix`,
// `bgra_to_i420`, `i420_to_bgra`, `nv12_to_bgra` and `FrameSize::chroma`
// (H.264 of SCR-M3).
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
// Not yet used by SCR-M2: `BeReader::remaining`.
#[allow(dead_code)]
pub(crate) mod wire;
pub(crate) mod wire_binary;
pub(crate) mod wire_control;
pub(crate) mod wire_hello;
