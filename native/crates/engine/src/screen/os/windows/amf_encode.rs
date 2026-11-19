//! AMF session of Windows hosts with an AMD GPU (BK12, B-22; HEVC and AV1
//! components by merge W1, names in `amf_codecs.rs`): the AMF runtime
//! `amfrt64.dll` is loaded at run time from System32, else from the driver
//! store folder of the AMD adapter by its full path
//! (`dxgi_adapters::driver_library`), the context is initialised with `InitDX11` on the
//! capture's D3D11 device, and every picture is the NV12 texture of BK2a's
//! ring wrapped with `CreateSurfaceFromDX11Native` (no copy; FFmpeg
//! `amfenc.c:698`, n7.1). Settings (B-21; R01 `video.cpp:977-1081`, R11):
//! usage "ultra low latency", `LowLatencyInternal`, quality preset "speed",
//! profile High, CBR with target = peak bit rate, VBV of one frame full at
//! the start, no B pictures, no periodic IDR (`IDRPeriod` 0, assumption
//! A-AMF-IDR), SPS and PPS with every IDR, HRD enforced, no filler data, no
//! skipped frames, BT.709 limited range. Bit rate and frame rate change at
//! run time through the dynamic properties (`VideoEncoderVCE.h:230-254`).
//! B-36 through two long-term references (`vendor_encode::LtrSlots`). A
//! value the encoder refuses stays at its default and is named once on the
//! host process's error output, as BK2a does for the MFT.
use std::ffi::c_void;
use std::io;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use libloading::os::windows::Library;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use super::amf_abi::{
    vtable, wide, AmfResult, AmfVariant, BufferVtbl, ComponentVtbl, ContextVtbl, DataVtbl,
    FactoryVtbl, InterfaceVtbl, StorageVtbl, This, AMF_DX11_0, AMF_FULL_VERSION, AMF_LIBRARY,
    AMF_MIN_VERSION, AMF_NEED_MORE_INPUT, AMF_OK, AMF_REPEAT, AMF_SURFACE_NV12, IID_AMF_BUFFER,
};
use super::amf_codecs::{names, Names};
use super::amf_settings::{
    dynamic_settings, key_settings, rate_settings, sharp_taken, static_settings, Setting,
};
use super::vendor_encode::{LtrSlots, RateParams, Recovery, VendorPicture, VendorSession};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::START_DIRECT_BPS;

pub(super) const AMF_NAME: &str = "amf";
/// Longest wait for the output of one picture (as `mf_encoder.rs`).
const OUTPUT_DEADLINE: Duration = Duration::from_secs(2);
/// Pause between two output polls when `QueryTimeout` is refused.
const OUTPUT_POLL: Duration = Duration::from_micros(250);

type InitFn = unsafe extern "C" fn(u64, *mut This) -> AmfResult;
type QueryVersionFn = unsafe extern "C" fn(*mut u64) -> AmfResult;

/// The AMF runtime of this process; the factory belongs to the DLL, which
/// stays loaded.
struct Runtime {
    factory: This,
    version: u64,
    _library: Library,
}

// SAFETY: the factory is the runtime's process-wide object (`AMFInit`);
// it is only used to create contexts and components (assumption
// A-AMF-THREADS: the factory is thread-safe).
unsafe impl Send for Runtime {}
// SAFETY: as above.
unsafe impl Sync for Runtime {}

/// Loads `name` from System32 only (task tests).
#[cfg(test)]
fn load_from(name: &str) -> io::Result<Runtime> {
    use libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32;
    // SAFETY: only System32 is searched, so no planted DLL loads.
    let library = unsafe { Library::load_with_flags(name, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|error| io::Error::new(io::ErrorKind::NotFound, format!("{name}: {error}")))?;
    initialised(library)
}

/// The AMF factory of the loaded runtime `library`; the initialisers of the
/// AMD runtime are the driver's own.
fn initialised(library: Library) -> io::Result<Runtime> {
    // SAFETY: `AMFInit_Fn` and `AMFQueryVersion_Fn` of `Factory.h:105-106`.
    let (init, query) = unsafe {
        let init = *library
            .get::<InitFn>(b"AMFInit\0")
            .map_err(io::Error::other)?;
        let query = *library
            .get::<QueryVersionFn>(b"AMFQueryVersion\0")
            .map_err(io::Error::other)?;
        (init, query)
    };
    let mut version = 0u64;
    // SAFETY: one writable u64.
    if unsafe { query(&mut version) } != AMF_OK || version < AMF_MIN_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("AMF runtime {version:#x} is older than 1.4"),
        ));
    }
    let mut factory = std::ptr::null_mut();
    // SAFETY: a writable factory slot; the requested version is at most
    // the runtime's own (decision: never ask a runtime for a newer API).
    let result = unsafe { init(version.min(AMF_FULL_VERSION), &mut factory) };
    if result != AMF_OK || factory.is_null() {
        return Err(io::Error::other(format!("AMFInit failed: {result}")));
    }
    Ok(Runtime {
        factory,
        version,
        _library: library,
    })
}

fn runtime() -> Result<&'static Runtime, String> {
    static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            crate::screen::os::dxgi_adapters::driver_library(super::VENDOR_AMD, AMF_LIBRARY)
                .and_then(initialised)
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Whether the AMF runtime loads in this process (no device needed).
pub(super) fn present() -> bool {
    runtime().is_ok()
}

/// Whether the runtime `name` loads from System32 (task tests).
#[cfg(test)]
pub(super) fn loads(name: &str) -> io::Result<()> {
    load_from(name).map(drop)
}

fn checked(result: AmfResult, what: &str) -> io::Result<()> {
    if result == AMF_OK {
        Ok(())
    } else {
        Err(io::Error::other(format!("AMF {what}: result {result}")))
    }
}

/// The object an AMF call created: success and a non-null reference.
fn created(result: AmfResult, object: This, what: &str) -> io::Result<This> {
    checked(result, what)?;
    if object.is_null() {
        return Err(io::Error::other(format!("AMF {what} gave no object")));
    }
    Ok(object)
}

/// An AMF object reference, released when dropped.
struct Owned(This);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: an owned reference of a live AMF interface.
            unsafe { (vtable::<InterfaceVtbl>(self.0).release)(self.0) };
        }
    }
}

/// A context or component: terminated, then released (FFmpeg
/// `amfenc.c:442-449`).
struct Terminated {
    object: This,
    terminate: unsafe extern "system" fn(This) -> AmfResult,
}

impl Drop for Terminated {
    fn drop(&mut self) {
        if !self.object.is_null() {
            // SAFETY: an owned, live context or component; nothing uses it
            // after this.
            unsafe {
                (self.terminate)(self.object);
                (vtable::<InterfaceVtbl>(self.object).release)(self.object);
            }
        }
    }
}

/// A context on `device` (`InitDX11`) and the encoder component `names` of
/// it; a runtime older than the component's minimum refuses.
fn context_and_encoder(
    device: &ID3D11Device,
    names: &Names,
) -> io::Result<(Terminated, Terminated)> {
    let runtime = runtime().map_err(io::Error::other)?;
    if runtime.version < names.min_version {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("AMF runtime too old for {}", names.component),
        ));
    }
    let factory = runtime.factory;
    // SAFETY: the runtime's live factory object.
    let calls = unsafe { vtable::<FactoryVtbl>(factory) };
    let mut raw = std::ptr::null_mut();
    // SAFETY: a writable slot for one new context reference.
    let raw = created(
        unsafe { (calls.create_context)(factory, &mut raw) },
        raw,
        "CreateContext",
    )?;
    // SAFETY: the reference written above is a live context.
    let terminate = unsafe { vtable::<ContextVtbl>(raw).terminate };
    let context = Terminated {
        object: raw,
        terminate,
    };
    // SAFETY: a live context; the device outlives the session (`_device`).
    let result =
        unsafe { (vtable::<ContextVtbl>(raw).init_dx11)(raw, device.as_raw(), AMF_DX11_0) };
    checked(result, "InitDX11")?;
    let name = wide(names.component);
    let mut raw = std::ptr::null_mut();
    // SAFETY: live factory and context; NUL-terminated component id.
    let result =
        unsafe { (calls.create_component)(factory, context.object, name.as_ptr(), &mut raw) };
    let raw = created(result, raw, "CreateComponent")?;
    // SAFETY: the reference written above is a live component.
    let terminate = unsafe { vtable::<ComponentVtbl>(raw).terminate };
    let component = Terminated {
        object: raw,
        terminate,
    };
    Ok((context, component))
}

fn set(storage: &StorageVtbl, this: This, name: &str, value: AmfVariant) -> bool {
    let name = wide(name);
    // SAFETY: a live AMF object; NUL-terminated name; value by copy.
    unsafe { (storage.set_property)(this, name.as_ptr(), value) == AMF_OK }
}

/// One AMF encoder of a fixed size on the capture device.
pub(super) struct AmfSession {
    // Dropped in this order: component, context, device.
    component: Terminated,
    context: Terminated,
    names: &'static Names,
    rate: RateParams,
    ltr: Option<LtrSlots>,
    frames: u64,
    refused: Vec<&'static str>,
    /// Level and encoder report of the bit-rate ceiling (r1, `amf_caps.rs`).
    ceiling: Option<u64>,
    _device: ID3D11Device,
}

impl AmfSession {
    pub(super) fn open(
        device: &ID3D11Device,
        codec: CodecKind,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Self> {
        Self::open_at(device, codec, size, fps, START_DIRECT_BPS, None)
    }

    /// `open` with the rate control starting at `bps` (plan 2026-11-17),
    /// with `quality` (P9, sharp mode) as quality VBR under that peak.
    pub(super) fn open_at(
        device: &ID3D11Device,
        codec: CodecKind,
        size: FrameSize,
        fps: u32,
        bps: u64,
        quality: Option<u8>,
    ) -> io::Result<Self> {
        let names = names(codec)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "AMF has no tiles"))?;
        let (context, component) = context_and_encoder(device, names)?;
        let rate = RateParams::for_target(bps, fps).with_quality(quality);
        let mut session = Self {
            component,
            context,
            names,
            rate,
            ltr: None,
            frames: 0,
            refused: Vec::new(),
            ceiling: None,
            _device: device.clone(),
        };
        let refused = session.apply(static_settings(names, size, &rate));
        sharp_taken(names, &rate, &refused)?;
        let this = session.component.object;
        // SAFETY: a live component; the size was checked by the factory.
        let result = unsafe {
            (vtable::<ComponentVtbl>(this).init)(
                this,
                AMF_SURFACE_NV12,
                size.width as i32,
                size.height as i32,
            )
        };
        checked(result, "Init")?;
        session.apply(dynamic_settings(names));
        session.ceiling = caps::session_ceiling(this, codec, size, fps);
        if !session.refused.contains(&names.max_ltr) {
            session.ltr = Some(LtrSlots::new(rate.fps));
        }
        Ok(session)
    }

    /// Sets `settings` on the component; refused ones stay at the default
    /// and are named once (`refused` keeps every name ever refused, for the
    /// log and `rate_refused`). Review 2026-11-19 C10: the names refused by
    /// this call, so a refusal of the current rate is seen.
    fn apply(&mut self, settings: Vec<Setting>) -> Vec<&'static str> {
        let this = self.component.object;
        // SAFETY: a live component.
        let storage = unsafe { &vtable::<ComponentVtbl>(this).storage };
        let (mut new, mut now) = (Vec::new(), Vec::new());
        for (name, value) in settings {
            if set(storage, this, name, value) {
                continue;
            }
            now.push(name);
            if !self.refused.contains(&name) {
                self.refused.push(name);
                new.push(name);
            }
        }
        if !new.is_empty() {
            eprintln!(
                "screen encoder {AMF_NAME} ({}): settings at the encoder's default: {}",
                self.names.component,
                new.join(", ")
            );
        }
        now
    }

    /// The access unit of `data` to `deliver`; whether it is a key frame.
    fn take_output(
        &self,
        data: This,
        deliver: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<bool> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: a live data object; one new buffer reference.
        let result = unsafe {
            (vtable::<InterfaceVtbl>(data).query_interface)(data, &IID_AMF_BUFFER, &mut raw)
        };
        let buffer = Owned(created(result, raw, "QueryInterface(AMFBuffer)")?);
        // SAFETY: a live buffer.
        let calls = unsafe { vtable::<BufferVtbl>(buffer.0) };
        // SAFETY: plain queries of the live buffer.
        let (native, length) =
            unsafe { ((calls.get_native)(buffer.0), (calls.get_size)(buffer.0)) };
        if native.is_null() || length == 0 {
            return Err(io::Error::other("AMF gave an empty access unit"));
        }
        // SAFETY: the buffer holds `length` bytes while it is referenced.
        deliver(unsafe { std::slice::from_raw_parts(native.cast::<u8>(), length) })?;
        let mut kind = AmfVariant::empty();
        let name = wide(self.names.output_type);
        // SAFETY: a live buffer; an integer property needs no clearing.
        let result =
            unsafe { (calls.data.storage.get_property)(buffer.0, name.as_ptr(), &mut kind) };
        Ok(result == AMF_OK && kind.as_int64() == Some(self.names.output_key))
    }
}

impl VendorSession for AmfSession {
    fn encode(
        &mut self,
        picture: VendorPicture<'_>,
        rate: &RateParams,
        deliver: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<bool> {
        let names = self.names;
        // Review 2026-11-19 C10: a lowered rate is set however small the
        // step, and a refused one fails the picture instead of running above.
        let lowered = self.rate.lowered_by(rate);
        if self.rate.differs(rate) || lowered {
            self.rate = *rate;
            let refused = self.apply(rate_settings(names, rate));
            if let Some(ltr) = self.ltr.as_mut() {
                ltr.set_fps(rate.fps);
            }
            // The peak counts in quality VBR (sharp mode) only; CBR may refuse it.
            let rated = [Some(names.target), rate.quality.map(|_| names.peak)];
            if let Some(name) = refused.iter().find(|name| rated.contains(&Some(**name))) {
                let error = io::Error::other(format!("AMF refused {name}"));
                rate.refused_lowering(lowered, AMF_NAME, &error)?;
            }
        }
        let context = self.context.object;
        let mut raw = std::ptr::null_mut();
        // SAFETY: a live context and a live texture of its device; one new
        // surface reference, no observer.
        let result = unsafe {
            (vtable::<ContextVtbl>(context).create_surface_from_dx11_native)(
                context,
                picture.texture.as_raw(),
                &mut raw,
                std::ptr::null_mut::<c_void>(),
            )
        };
        let surface = Owned(created(result, raw, "CreateSurfaceFromDX11Native")?);
        // SAFETY: the surface's AMFData entries.
        let data = unsafe { vtable::<DataVtbl>(surface.0) };
        let duration = 10_000_000 / i64::from(self.rate.fps.max(1));
        // SAFETY: plain writes on the live surface.
        unsafe {
            (data.set_pts)(surface.0, self.frames as i64 * duration);
            (data.set_duration)(surface.0, duration);
        }
        let mut per_picture = Vec::new();
        if picture.idr {
            per_picture.extend(key_settings(names));
        }
        if let Some(ltr) = self.ltr.as_mut() {
            let plan = ltr.plan(picture.frame_id, picture.idr);
            if let Some(index) = plan.mark {
                per_picture.push((names.mark_ltr, AmfVariant::int64(i64::from(index))));
            }
            if let Some(bits) = plan.force {
                per_picture.push((names.force_ltr, AmfVariant::int64(i64::from(bits))));
            }
        }
        // Only the key frame request is required; the parameter sets of a
        // key frame are checked by `vendor_encode` (and taken from the last
        // ones when missing), a refused LTR property ends B-36 for this
        // session (key frames after a reset, as today).
        for (name, value) in per_picture {
            if set(&data.storage, surface.0, name, value) {
                continue;
            }
            if name == names.force_type {
                return Err(io::Error::other("AMF refused the key frame request"));
            }
            if name == names.mark_ltr || name == names.force_ltr {
                self.ltr = None;
            }
        }
        let component = self.component.object;
        // SAFETY: the component's entries.
        let calls = unsafe { vtable::<ComponentVtbl>(component) };
        // SAFETY: a live component and surface; the component takes its own
        // reference (FFmpeg releases right after `SubmitInput`, `:779-785`).
        checked(
            unsafe { (calls.submit_input)(component, surface.0) },
            "SubmitInput",
        )?;
        drop(surface);
        self.frames += 1;
        let deadline = Instant::now() + OUTPUT_DEADLINE;
        loop {
            let mut raw = std::ptr::null_mut();
            // SAFETY: a live component; one new data reference or none.
            let result = unsafe { (calls.query_output)(component, &mut raw) };
            if !raw.is_null() {
                let output = Owned(raw);
                return self.take_output(output.0, deliver);
            }
            if ![AMF_OK, AMF_REPEAT, AMF_NEED_MORE_INPUT].contains(&result) {
                return Err(io::Error::other(format!(
                    "AMF QueryOutput: result {result}"
                )));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the AMF encoder did not answer in time",
                ));
            }
            std::thread::sleep(OUTPUT_POLL);
        }
    }

    fn recover_references(&mut self, last_good: u64) -> Recovery {
        // `recover` changes the slots, so it runs outside a match guard
        // (a guard borrows immutably).
        if self.ltr.as_mut().is_some_and(|ltr| ltr.recover(last_good)) {
            Recovery::Continued
        } else {
            Recovery::KeyframeNeeded
        }
    }

    fn rate_ceiling(&self, _fps: u32) -> Option<u64> {
        self.ceiling
    }

    /// The component refused its target bit rate, at the start or since
    /// (`apply`, plan 2026-11-17): it runs at its own default.
    fn rate_refused(&self) -> bool {
        self.refused.contains(&self.names.target)
    }
}

// `probe`, `refused_settings` and the session's rate ceiling (r1).
#[path = "amf_caps.rs"]
pub(super) mod caps;
pub(super) use caps::{probe, refused_settings};
