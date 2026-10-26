//! Debug-only fixtures use the real process pool and production capture authority.
use std::{collections::BTreeMap, io, sync::{Arc, Mutex}, time::Duration};
use serde_json::{json, Value};
use crate::screen::core::{
    capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, CaptureSlice, SessionAdmission, SourceKey},
    capture_plan::CapturePhases,
    frame_budget::{FrameContract, FrameLimits, FRAME_PROTOCOL, VERSIONED_WORKING_IMAGES},
    frame_memory::{self, CapacityCeiling}, frame_size_caps::{PacketBudget, RECORD_BUFFER_OVERHEAD},
    host_codec_adapters::{HostContract, HostCodecPlan, HostOutputPlan}, pixel_convert::FrameSize,
};
use super::{bridge_channels as channels, android_capture_admission as capture, android_capture_arena as arena};

struct Viewer {
    reader: Option<CaptureSlice>, pull: u64, reader_bytes: u64, pull_bytes: usize,
    grant: CaptureGrant, admission: Arc<SessionAdmission>,
}
impl Drop for Viewer {
    fn drop(&mut self) {arena::release(self.pull);}
}
struct Fixture {
    channel: Arc<channels::Channel>, size: FrameSize, output: FrameSize, kind: u32,
    heap: u64, retired: bool, generations: BTreeMap<u64, CaptureGrant>, viewer: Option<Viewer>,
    // Diagnostic aliases intentionally hold host W through successful finish.
    admission: Arc<SessionAdmission>,
}
static FIXTURES: Mutex<BTreeMap<u64, Fixture>> = Mutex::new(BTreeMap::new());
fn invalid(reason: &str) -> io::Error {io::Error::other(reason)}
fn registry() -> io::Result<std::sync::MutexGuard<'static, BTreeMap<u64, Fixture>>> {
    FIXTURES.lock().map_err(|_| invalid("capture fixture registry unavailable"))
}
fn get(entries: &mut BTreeMap<u64, Fixture>, id: u64) -> io::Result<&mut Fixture> {
    entries.get_mut(&id).ok_or_else(|| invalid("unknown capture fixture session"))
}
fn admission(key: &str, heap: u64, contract: FrameContract) -> io::Result<Arc<SessionAdmission>> {
    let admitted = frame_memory::admit_session(None, CapacityCeiling::bounded(heap), contract.limits.working_bytes, key)?;
    if admitted.working_bytes() != contract.limits.working_bytes {return Err(invalid("heap cannot carry the complete fixture W"));}
    admitted.bind_contract(Some(contract))?;
    Ok(admitted)
}
fn viewer(id: u64, heap: u64, contract: FrameContract, size: FrameSize) -> io::Result<Viewer> {
    let key = format!("android-image-fixture-viewer-{id}");
    let admission = admission(&key, heap, contract)?;
    let source = SourceKey::new(&key, "fixture-decoder")?;
    let packet = PacketBudget::for_contract(Some(contract))?;
    let yuv = size.pixels().checked_add(size.chroma().pixels().checked_mul(2).ok_or_else(|| invalid("YUV overflow"))?)
        .and_then(|n| n.checked_mul(3)).ok_or_else(|| invalid("reader byte overflow"))?;
    let mut plan = CapturePlan::new(source, CaptureGeneration::new(1));
    plan.bind_contract(Some(contract))?;
    plan.add_bytes(CaptureOwner::NativeSurface, yuv, 1, CapturePhases::ALL)?;
    plan.add_bytes(CaptureOwner::Transport, packet.pull_bytes as u64, 1, CapturePhases::ALL)?;
    let grant = admission.authority().approve(plan)?;
    let reader = grant.claim(CaptureOwner::NativeSurface, yuv)?;
    let pull = arena::claim(&grant, CaptureOwner::Transport as u32, packet.pull_bytes as u64, true)?;
    Ok(Viewer {reader: Some(reader), pull, reader_bytes: yuv, pull_bytes: packet.pull_bytes, grant, admission})
}
pub(crate) fn open(size: FrameSize, output: FrameSize, kind: u32, heap: u64, decoder: bool) -> io::Result<Value> {
    if heap == 0 || !matches!(kind, capture::ACCESSIBILITY | capture::PROJECTION_TILES | capture::SYNTHETIC_SURFACE)
        || (decoder && kind != capture::SYNTHETIC_SURFACE) {return Err(invalid("invalid fixture heap, kind or decoder role"));}
    FrameLimits::RESOURCE.check(size)?;
    FrameLimits::RESOURCE.check(output)?;
    if (kind != capture::ACCESSIBILITY && output != size)
        || (kind == capture::ACCESSIBILITY && output != FrameSize::new((size.width / 4).max(1), (size.height / 4).max(1))) {
        return Err(invalid("fixture capture/output geometry mismatch"));
    }
    let working = output.pixels().checked_mul(4).and_then(|n| n.checked_mul(VERSIONED_WORKING_IMAGES))
        .ok_or_else(|| invalid("fixture W overflow"))?;
    let contract = FrameContract {version: FRAME_PROTOCOL, limits: FrameLimits {
        max_size: output, max_pixels: output.pixels(), working_bytes: working}, hybrid_regions: false};
    contract.validate()?;
    crate::screen::os::image_memory::observe_capacity_ceiling(CapacityCeiling::bounded(heap))?;
    let mut entries = registry()?;
    let channel = channels::open_fixture(channels::HostStart {source: "synthetic".into(), still: kind == capture::ACCESSIBILITY,
        viewer_device_id: "android-image-fixture".into(), viewer_name: "Android instrumentation".into(),
        max_width: output.width, max_height: output.height});
    let id = channel.id;
    let result = (|| {
        let key = format!("android-image-fixture-host-{id}");
        let admission = admission(&key, heap, contract)?;
        let source = SourceKey::new(&key, "fixture-capture")?;
        let codec = if kind == capture::SYNTHETIC_SURFACE {HostCodecPlan::OpenH264} else {HostCodecPlan::Tiles};
        let output_plan = HostOutputPlan::new(HostContract::Versioned(contract), output, codec)?;
        channel.capture.attach(admission.clone(), source)?;
        let plan = channel.capture.plan(size, kind, &output_plan)?;
        let grant = admission.authority().approve(plan)?;
        channel.capture.activate(grant.clone())?;
        let viewer = if decoder {Some(viewer(id, heap, contract, size)?)} else {None};
        let mut fixture = Fixture {channel: channel.clone(), size, output, kind, heap, retired: false,
            generations: BTreeMap::from([(grant.generation().value(), grant)]), viewer, admission};
        let value = describe(&mut fixture)?;
        entries.insert(id, fixture);
        Ok(value)
    })();
    if result.is_err() {channels::close_fixture(id);}
    result
}
fn describe(fixture: &mut Fixture) -> io::Result<Value> {
    let current = fixture.channel.capture.current().map(|grant| grant.generation().value()).unwrap_or(0);
    let generations = fixture.generations.iter().map(|(generation, grant)|
        Ok(json!({"generation":generation,"live_bytes":grant.live_bytes()?}))).collect::<io::Result<Vec<_>>>()?;
    let peak = fixture.generations.values().map(|grant| grant.plan().peak_bytes()).max().unwrap_or(0);
    let mut value = json!({"session":fixture.channel.id,"generation":current,"phase":if fixture.retired {"retired"} else {"active"},
        "width":fixture.size.width,"height":fixture.size.height,"output_width":fixture.output.width,"output_height":fixture.output.height,
        "kind":fixture.kind,"heap_offer":fixture.heap,"working_bytes":fixture.admission.working_bytes(),
        "additional_bytes":fixture.admission.additional_bytes()?,"plan_peak_bytes":peak,"generations":generations,
        "decoder_reader_bytes":fixture.viewer.as_ref().filter(|v|v.reader.is_some()).map_or(0,|v|v.reader_bytes),
        "viewer_working_bytes":fixture.viewer.as_ref().map_or(0,|v|v.admission.working_bytes()),
        "viewer_pull_bytes":fixture.viewer.as_ref().map_or(0,|v|v.pull_bytes)});
    if let Some(viewer) = &fixture.viewer {
        let contract = FrameContract {version: FRAME_PROTOCOL, limits: FrameLimits {max_size:fixture.output,
            max_pixels:fixture.output.pixels(), working_bytes:viewer.admission.working_bytes()}, hybrid_regions:false};
        let packet = PacketBudget::for_contract(Some(contract))?;
        value["viewer_budget"] = json!({"workingBytes":viewer.admission.working_bytes(),"imageBytes":packet.raw_bytes-8,
            "recordBytes":packet.record_bytes,"packetBytes":packet.encoded_bytes,"pullBytes":packet.pull_bytes,
            "legacy":false,"recordOverheadBytes":RECORD_BUFFER_OVERHEAD});
    }
    Ok(value)
}
/// Called only after production begin succeeded. This never creates a grant.
pub(crate) fn observed_begin(id: u64, generation: u64) {
    let Ok(mut entries) = registry() else {return;};
    let Some(fixture) = entries.get_mut(&id).filter(|fixture| !fixture.retired) else {return;};
    if let Ok(grant) = fixture.channel.capture.grant(generation) {
        fixture.generations.retain(|key, old| *key == generation || old.live_bytes().map_or(true, |bytes| bytes != 0));
        fixture.generations.insert(generation, grant);
    }
}
pub(crate) fn state(id: u64) -> io::Result<Value> {describe(get(&mut *registry()?, id)?)}
pub(crate) fn consume_still(id: u64, generation: u64) -> io::Result<Value> {
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    if fixture.retired {return Err(invalid("capture fixture is retired"));}
    let grant = fixture.generations.get(&generation).ok_or_else(|| invalid("unobserved capture fixture generation"))?;
    let picture = fixture.channel.next_picture(Duration::ZERO, |_| true).map_err(io::Error::other)?.ok_or_else(|| invalid("capture fixture picture missing"))?;
    let bytes = picture.data.as_slice();
    let expected = fixture.size.checked_bgra_len()?.checked_add(8).ok_or_else(|| invalid("still byte overflow"))?;
    let held = picture.data.grant()?;
    if !picture.still() || bytes.len() != expected || bytes.get(..4) != Some(fixture.size.width.to_le_bytes().as_slice())
        || bytes.get(4..8) != Some(fixture.size.height.to_le_bytes().as_slice())
        || held.book_identity() != grant.book_identity() || held.generation() != grant.generation() || held.source() != grant.source() {
        return Err(invalid("capture fixture still bytes or owner mismatch"));
    }
    Ok(json!({"generation":generation,"width":fixture.size.width,"height":fixture.size.height,"length":bytes.len()}))
}
pub(crate) fn pull_address(id: u64) -> io::Result<(usize, usize)> {
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    if fixture.retired {return Err(invalid("capture fixture is retired"));}
    let viewer = fixture.viewer.as_ref().ok_or_else(|| invalid("fixture has no decoder viewer"))?;
    arena::address(viewer.pull).ok_or_else(|| invalid("fixture pull allocation missing"))
}
pub(crate) fn release_reader(id: u64) -> io::Result<()> {
    get(&mut *registry()?, id)?.viewer.as_mut().ok_or_else(|| invalid("fixture has no decoder reader"))?.reader.take();
    Ok(())
}
pub(crate) fn release_viewer(id: u64) -> io::Result<()> {
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    let viewer = fixture.viewer.as_ref().ok_or_else(|| invalid("fixture viewer already released"))?;
    if viewer.reader.is_some() {return Err(invalid("decoder reader still owns its claim"));}
    // Caller has observed onEnded and relinquished every Kotlin pull view.
    arena::release(viewer.pull);
    viewer.grant.retire()?;
    if viewer.grant.live_bytes()? != 0 {return Err(invalid("decoder viewer still owns live claims"));}
    fixture.viewer = None;
    Ok(())
}
pub(crate) fn retire(id: u64) -> io::Result<()> {
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    if !fixture.retired {fixture.retired = true; channels::close_fixture(id);}
    Ok(())
}
pub(crate) fn finish(id: u64) -> io::Result<()> {
    let mut entries = registry()?;
    let fixture = get(&mut entries, id)?;
    if !fixture.retired || fixture.viewer.is_some() || channels::channel(id).is_some()
        || fixture.generations.values().any(|grant| grant.live_bytes().map_or(true, |bytes|bytes != 0)) {
        return Err(invalid("capture fixture owners have not actually ended"));
    }
    entries.remove(&id);
    Ok(())
}
