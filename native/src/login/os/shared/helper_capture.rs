//! Delegation over the already verified parent channel, never a child pool.
use std::{io::{self, Read, Write}, sync::{Arc, Mutex, Weak, mpsc::{self, Receiver}}, time::Duration};
use crate::{login::core::{capture_wire::{CaptureCall, GrantReply, Plan}, wire::{self, HelperCall, HelperCommand, HelperInit, Reply, CAPTURE, PIXELS}},
    screen::core::{capture_admission::{CaptureAuthority, CaptureGrant, CapturePlan, SessionAdmission},
        capture_grant::DelegatedCaptureCredit, capture_image::CapturedImage,
        capture_source_contract::{HostContract, HostOutputPlan, PreparedSource}, host_backend::ScreenSource}};

pub(crate) type Output = Arc<Mutex<Box<dyn Write + Send>>>;
struct Retirement {id: u64, descriptor: Plan, cookie: String, output: Output}
impl Drop for Retirement {
    fn drop(&mut self) {
        // No ACK is guessed locally. A lost retirement leaves the real Parent
        // promise intact until confirmed process and reader end.
        let _ = send_capture(&self.output, &CaptureCall::Retire {cookie: self.cookie.clone(), id: self.id});
    }
}
struct State {next: u64, replies: Receiver<GrantReply>, live: Vec<Weak<Retirement>>}
pub(crate) struct HelperCapture {cookie: String, output: Output, state: Mutex<State>}
impl HelperCapture {
    pub(crate) fn open(init: &HelperInit, mut input: Box<dyn Read + Send>, output: Output)
        -> io::Result<(Arc<Self>, Receiver<io::Result<HelperCall>>, Box<dyn ScreenSource>)> {
        let descriptor = init.capture.as_ref().ok_or_else(|| super::denied("login helper has no parent capture authority"))?;
        if descriptor.session != init.cookie {return Err(super::denied("login capture admission differs from inherited capability"));}
        let (grant_sender, replies) = mpsc::sync_channel(1);
        let (sender, commands) = mpsc::sync_channel(1);
        let cookie = init.cookie.clone();
        std::thread::Builder::new().name("login-helper-input".into()).spawn(move || loop {
            let call = wire::receive::<HelperCall>(&mut input).and_then(|call| {
                if call.cookie != cookie {return Err(super::denied("wrong inherited login helper capability"));}
                Ok(call)
            });
            match call {
                Ok(HelperCall {command: HelperCommand::Capture(reply), ..}) => {
                    if grant_sender.send(reply).is_err() {break;}
                }
                Ok(call) => {if sender.send(Ok(call)).is_err() {break;}},
                Err(error) => {let _ = sender.send(Err(error)); break;},
            }
        })?;
        let authority = Arc::new(Self {cookie: init.cookie.clone(), output,
            state: Mutex::new(State {next: 0, replies, live: Vec::new()})});
        let admission = Arc::new(SessionAdmission::from_verified_parent_unbound(&descriptor.session, descriptor.working,
            authority.clone(), authority.output.clone())?);
        let prepared = crate::screen::os::prepare_login_source(admission.clone())?;
        let source = activate_prepared(prepared, admission)?;
        Ok((authority, commands, source))
    }
    pub(crate) fn pixels(&self, output: &Output, image: &CapturedImage) -> io::Result<()> {
        if image.source().session() != self.cookie {return Err(super::denied("login image belongs to another inherited session"));}
        let size = image.size();
        let bytes = image.image().data();
        if bytes.len() != crate::login::core::capture_wire::raw_length(size.width, size.height)? {
            return Err(super::denied("login capture exceeds bounded raw frame"));
        }
        let owner = {
            let state = self.state.lock().map_err(|_| io::Error::other("login capture delegation poisoned"))?;
            state.live.iter().filter_map(Weak::upgrade).find(|owner|
                owner.descriptor.raw_bytes != 0 && owner.descriptor.source == image.source().source()
                    && owner.descriptor.generation == image.generation().value())
                .ok_or_else(|| super::denied("login image has no live inherited grant"))?
        };
        if bytes.len() as u64 > owner.descriptor.raw_bytes {return Err(super::denied("login image exceeds parent response credit"));}
        let mut reply = Reply::state("pixels");
        reply.width = size.width; reply.height = size.height; reply.pixel_bytes = bytes.len(); reply.capture_id = Some(owner.id);
        let mut writer = output.lock().map_err(|_| io::Error::other("login helper output poisoned"))?;
        wire::send(&mut *writer, &reply)?;
        wire::write_frame(&mut *writer, PIXELS, bytes)
    }
}
impl CaptureAuthority for HelperCapture {
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant> {
        let descriptor = Plan::from_plan(&plan)?;
        let mut state = self.state.lock().map_err(|_| io::Error::other("login capture delegation poisoned"))?;
        state.live.retain(|owner| owner.strong_count() != 0);
        let main_exists = state.live.iter().filter_map(Weak::upgrade).any(|owner|
            owner.descriptor.raw_bytes != 0 && owner.descriptor.source == descriptor.source
                && owner.descriptor.generation == descriptor.generation);
        if (descriptor.raw_bytes != 0 && main_exists) || (descriptor.raw_bytes == 0 && !main_exists) {
            return Err(super::denied("login capture main/supplemental generation binding differs"));
        }
        state.live.try_reserve(1).map_err(|_| io::Error::other("login capture delegation allocation failed"))?;
        let id = state.next.checked_add(1).ok_or_else(|| super::denied("login capture request sequence exhausted"))?;
        state.next = id;
        send_capture(&self.output, &CaptureCall::Prepare {cookie: self.cookie.clone(), id, plan: descriptor.clone()})?;
        let reply = state.replies.recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "parent capture authority did not answer"))?;
        if reply.id != id || !reply.plan.matches(&descriptor) {
            return Err(super::denied("parent capture reply differs from requested source/generation/limits"));
        }
        if !reply.approved {return Err(io::Error::other("parent refused login capture admission"));}
        let owner = Arc::new(Retirement {id, descriptor, cookie: self.cookie.clone(), output: self.output.clone()});
        let credit = DelegatedCaptureCredit::from_verified_parent(plan.source().clone(), plan.generation(),
            plan.peak_bytes(), plan.owner_limits(), owner.clone())?;
        let grant = CaptureGrant::from_delegated(plan, credit)?;
        state.live.push(Arc::downgrade(&owner));
        Ok(grant)
    }
}
/// Shared by the real helper and its metadata-only adapter fixture. Neither
/// geometry checks nor a successful prepare are activation permission.
pub(crate) fn activate_prepared(prepared: Box<dyn PreparedSource>, admission: Arc<SessionAdmission>)
    -> io::Result<Box<dyn ScreenSource>> {
    let info = prepared.description();
    let size = info.native_size.ok_or_else(|| io::Error::other("login native geometry is pending"))?;
    crate::login::core::capture_wire::raw_length(size.width, size.height)?;
    let output = HostOutputPlan::for_source(HostContract::Legacy, size, info.capture_limits)?;
    admission.bind_contract(None)?;
    let plan = prepared.capture_source_plan(&output)?.ok_or_else(|| io::Error::other("login capture grant metadata is pending"))?;
    let grant = admission.authority().approve(plan)?;
    prepared.activate(grant).map_err(|reason| io::Error::other(reason.reason))
}
pub(crate) fn send(output: &Output, reply: &Reply) -> io::Result<()> {
    let mut writer = output.lock().map_err(|_| io::Error::other("login helper output poisoned"))?;
    wire::send(&mut *writer, reply)
}
fn send_capture(output: &Output, call: &CaptureCall) -> io::Result<()> {
    let bytes = serde_json::to_vec(call).map_err(io::Error::other)?;
    let mut writer = output.lock().map_err(|_| io::Error::other("login helper output poisoned"))?;
    wire::write_frame(&mut *writer, CAPTURE, &bytes)
}
