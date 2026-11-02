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
//! SCR-M4 brings `snapshot` (the still picture: PNG, fitting a long edge and a
//! byte budget, and the picture of a `snapshot` stream).
//!
//! Parts of SCR-M1 that SCR-M2 and SCR-M3 do not use yet keep
//! `allow(dead_code)` on their module; each such declaration names them.

pub mod app_icon;
pub mod authorize;
#[cfg(test)]
mod batches_4k_task_tests;
#[cfg(test)]
mod batches_capture_admission_task_tests;
#[cfg(test)]
mod batches_capture_image_task_tests;
#[cfg(test)]
mod batches_capture_import_task_tests;
#[cfg(test)]
mod batches_image_admission_task_tests;
#[cfg(test)]
pub mod batches_quality_acceptance_task_tests;
pub mod bench;
pub mod capture_admission;
pub mod capture_bytes;
pub mod capture_grant;
pub mod capture_image;
pub mod capture_import;
pub mod capture_plan;
pub mod capture_records;
pub mod capture_source_contract;
pub mod capture_transport;
pub mod catalog_entries;
pub mod codec_registry;
pub mod control_sender;
pub mod decoded_frame;
pub mod feed_host;
pub mod frame_budget;
pub mod frame_memory;
pub mod frame_protocol;
pub mod frame_reference;
pub mod frame_regions;
pub mod frame_size_caps;
pub mod gop_stream;
pub mod host_admission;
pub mod host_backend;
pub mod host_codec_adapters;
pub mod host_hybrid;
pub mod host_metadata;
pub mod host_metadata_jobs;
pub mod host_pipeline;
pub mod host_server;
pub mod host_stage;
pub mod icon_cache;
pub mod icon_host;
pub mod image_admission;
#[cfg(any(test, feature = "test-support"))]
pub mod quality_acceptance_baseline;
#[cfg(any(test, feature = "test-support"))]
pub mod quality_acceptance_metrics;
pub mod visibility;
// Not yet used by SCR-M2: `TOOLBAR_KEYS`, `key_usage`, `key_name`,
// `pointer_to_pixel`, `pixel_to_pointer` and `PressedInputs::is_empty`
// (input of SCR-M6 and the viewer of SCR-M7).
#[allow(dead_code)]
pub mod input_model;
pub mod licenses;
// Not yet used by SCR-M2: `LatencySamples::count`.
#[allow(dead_code)]
pub mod metrics;
pub mod negotiate;
// Not yet used: `Pacer::frames_in_flight` (no field of `STATS` carries it;
// the viewer statistics of SCR-M7 are its consumer).
#[allow(dead_code)]
pub mod pacing;
// Used only on Windows so far: `Nv12View` and `nv12_to_bgra` (the Media
// Foundation decoder of SCR-M3); the desktop viewer of SCR-M7 uses them on
// every desktop.
#[allow(dead_code)]
pub mod pixel_convert;
// Not yet used by SCR-M2: `RateController::receive_rate_bps`.
#[allow(dead_code)]
pub mod rate_control;
pub mod registry;
pub mod sender_budget;
// Not yet used by SCR-M2: `SessionTracker`, `SessionState::can_move_to`,
// `SessionState::is_terminal` and `KeyframeRequester::is_open`.
#[allow(dead_code)]
pub mod session_state;
pub mod snapshot;
pub mod synthetic;
pub mod tiles_codec;
pub mod viewer_client;
pub mod viewer_reconstruct;
pub mod window_list;
pub mod wire;
pub mod wire_binary;
pub mod wire_control;
pub mod wire_hello;
pub mod worker_capture_wire;
pub mod worker_wire;

pub mod family_listing;
pub mod family_protocol;
pub mod family_viewer;
pub mod worker_family;

#[cfg(any(test, feature = "test-support"))]
pub mod capture_test_support;

#[cfg(any(test, feature = "test-support"))]
pub mod screen_metadata_runtime_fixtures;

#[cfg(test)]
pub mod batches_capture_pipeline_task_tests;

#[cfg(test)]
pub mod batches_capture_worker_task_tests;
