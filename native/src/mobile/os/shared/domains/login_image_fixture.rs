//! Debug-only ViewerFrame source, deliberately absent from authenticated input sessions.
use crate::{
    login::{FrameToken, ViewerFrame},
    screen::core::{
        capture_admission::{CaptureGeneration, SourceKey},
        capture_image::CaptureImageBuilder,
        frame_budget::FrameLimits,
        image_admission::{ImageAdmission, ImageRole},
        pixel_convert::FrameSize,
    },
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
struct Fixture {
    owner: u64,
    size: FrameSize,
    closed: bool,
    queued: Option<ViewerFrame>,
    generation: CaptureGeneration,
    images: Arc<ImageAdmission>,
    pane: Arc<super::login_images::Pane>,
}
static FIXTURES: Mutex<BTreeMap<u64, Fixture>> = Mutex::new(BTreeMap::new());
fn invalid(reason: &str) -> io::Error {
    io::Error::other(reason)
}
fn registry() -> io::Result<std::sync::MutexGuard<'static, BTreeMap<u64, Fixture>>> {
    FIXTURES
        .lock()
        .map_err(|_| invalid("login fixture registry unavailable"))
}
fn get(entries: &mut BTreeMap<u64, Fixture>, id: u64) -> io::Result<&mut Fixture> {
    entries
        .get_mut(&id)
        .ok_or_else(|| invalid("unknown login fixture session"))
}
pub(super) fn open(owner: u64, heap: u64, width: u32, height: u32) -> io::Result<u64> {
    let size = FrameSize::new(width, height);
    FrameLimits::LEGACY.check(size)?;
    let mut entries = registry()?;
    if entries.len() >= 64 || entries.values().any(|fixture| fixture.owner == owner) {
        return Err(invalid(
            "login fixture owner already registered or registry full",
        ));
    }
    let pane = super::login_images::fixture_pane(owner)?;
    let state = super::login_images::fixture_pane_state(owner, &pane)?;
    if state["session"].as_u64() != Some(0) || state["closed"].as_bool() != Some(false) {
        return Err(invalid(
            "login fixture requires an unbound live product pane",
        ));
    }
    let id = super::login_view::fixture_session_id().map_err(io::Error::other)?;
    let images = super::login_images::bind(owner, id, heap)?;
    let generation = super::login_images::fixture_state(owner)?["generation"]
        .as_u64()
        .ok_or_else(|| invalid("login fixture generation missing"))?;
    entries.insert(
        id,
        Fixture {
            owner,
            size,
            closed: false,
            queued: None,
            generation: CaptureGeneration::new(generation),
            images,
            pane,
        },
    );
    Ok(id)
}
pub(super) fn enqueue(id: u64, frame_id: u64, pattern: u32) -> io::Result<()> {
    if frame_id == 0 || pattern != 0 {
        return Err(invalid("invalid login fixture frame or pattern"));
    }
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    if fixture.closed {
        return Err(invalid("login fixture is closed"));
    }
    // Replacing an unconsumed candidate releases its actual raw slot first.
    fixture.queued = None;
    let source = SourceKey::new(
        &format!("android-login-fixture-{id}"),
        "synthetic-viewer-frame",
    )?;
    let mut image = CaptureImageBuilder::new_local(
        fixture.size,
        FrameLimits::LEGACY,
        &fixture.images,
        ImageRole::ViewerInput,
        source,
        fixture.generation,
    )?;
    for (index, pixel) in image.pixels_mut().chunks_exact_mut(4).enumerate() {
        let x = index as u32 % fixture.size.width;
        let y = index as u32 / fixture.size.width;
        // Different values in every channel and row prove BGRA->RGBA and stride.
        pixel.copy_from_slice(&[
            (17 + (x * 19 + y * 7) % 200) as u8,
            (31 + (x * 11 + y * 23) % 200) as u8,
            (47 + (x * 29 + y * 13) % 200) as u8,
            255,
        ]);
    }
    // A synthetic identity only: never inserted into ViewerSession/ViewerGuard.
    let token = FrameToken {
        frame_id,
        width: fixture.size.width,
        height: fixture.size.height,
        digest: format!(
            "{id:016x}{frame_id:016x}{:016x}{:016x}",
            fixture.size.width, fixture.size.height
        ),
    };
    fixture.queued = Some(ViewerFrame {
        token,
        image: image.freeze(),
    });
    Ok(())
}
pub(super) fn take(id: u64) -> Option<Option<ViewerFrame>> {
    let mut entries = registry().ok()?;
    let fixture = entries.get_mut(&id)?;
    Some(if fixture.closed {
        None
    } else {
        fixture.queued.take()
    })
}
pub(super) fn close(id: u64) {
    if let Ok(mut entries) = registry() {
        if let Some(fixture) = entries.get_mut(&id) {
            fixture.closed = true;
            fixture.queued = None;
        }
    }
}
pub(super) fn state(id: u64) -> io::Result<Value> {
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    let mut state = super::login_images::fixture_pane_state(fixture.owner, &fixture.pane)?;
    state["fixture_session"] = json!(id);
    state["queued"] = json!(fixture.queued.is_some());
    state["fixture_closed"] = json!(fixture.closed);
    Ok(state)
}
pub(super) fn finish(id: u64) -> io::Result<()> {
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    if !fixture.closed || fixture.queued.is_some() || fixture.images.live_bytes()? != 0 {
        return Err(invalid(
            "login fixture image owners have not actually ended",
        ));
    }
    entries.remove(&id);
    Ok(())
}
