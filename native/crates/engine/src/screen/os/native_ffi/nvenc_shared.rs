//! NVENC as both desktop platforms use it (merge W2, plan 2026-11-14
//! „Bildkette“): the structure layouts (`abi`), the interface without a
//! device (`api`: versions, constants, function list, error text, capability
//! queries), the HEVC and AV1 views of the codec union (`codecs`) and the
//! encoder settings of B-21 and B-36 (`settings`). Platform code keeps only
//! what differs: loading the driver library and the device of a session
//! (Linux: `libnvidia-encode.so.1` with a CUDA context and NV12 input
//! buffers, `linux_os/nvenc_*`; Windows: `nvEncodeAPI64.dll` with the D3D11
//! device and registered textures, `windows/nvenc_*`). FFI, so under `os/`
//! and not under `core/`.
#[path = "nvenc_abi.rs"]
pub(in crate::screen::os) mod abi;
#[path = "nvenc_api.rs"]
pub(in crate::screen::os) mod api;
#[path = "nvenc_codecs.rs"]
pub(in crate::screen::os) mod codecs;
#[path = "nvenc_settings.rs"]
pub(in crate::screen::os) mod settings;
