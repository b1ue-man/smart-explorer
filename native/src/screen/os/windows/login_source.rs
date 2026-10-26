//! An immutable output/desktop binding inside the verified login helper.
use std::io;
use std::sync::{Arc,Mutex,Weak};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput,INPUT};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics,SM_CXVIRTUALSCREEN,SM_CYVIRTUALSCREEN,SM_XVIRTUALSCREEN,SM_YVIRTUALSCREEN};
use super::capture_dxgi::{DxgiCapture,OutputBinding,PreparedDxgi};
use super::capture_slots::{CaptureBudget,CaptureGeometry};
use crate::screen::core::capture_admission::{CaptureGeneration,CaptureGrant,SessionAdmission,SourceKey};
use crate::screen::core::capture_source_contract::{HostOutputPlan,PreparedSource,PreparedSourceInfo};
use crate::screen::core::capture_plan::CapturePlan;
use crate::screen::core::capture_image::CapturedImage;
use crate::screen::core::host_backend::SourceRefusal;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::session_state::SessionState;
use super::capture_source::Grabber;
use super::input_send::Area;
use super::input_keys::SasGuard;
use crate::screen::core::host_backend::{ScreenSource,SourceFrame};
use crate::screen::core::input_model::{InputEvent,PointerButton,TouchPhase};
use crate::screen::core::pixel_convert::FrameSize;

static INPUT:Mutex<Option<Weak<Mutex<Context>>>>=Mutex::new(None);
fn expired()->io::Error {io::Error::new(io::ErrorKind::PermissionDenied,"login capture geometry changed; start a new login connection")}
fn desktop()->Area {
    // SAFETY: read-only metrics on the helper's verified input desktop.
    unsafe {Area {left:GetSystemMetrics(SM_XVIRTUALSCREEN),top:GetSystemMetrics(SM_YVIRTUALSCREEN),
        width:GetSystemMetrics(SM_CXVIRTUALSCREEN),height:GetSystemMetrics(SM_CYVIRTUALSCREEN)}}
}
trait LoginCapture:Grabber {fn output_binding(&self)->io::Result<OutputBinding>;}
impl LoginCapture for DxgiCapture {
    fn output_binding(&self)->io::Result<OutputBinding> {DxgiCapture::output_binding(self)}
}
struct Context<C=DxgiCapture> {
    capture:C,output:OutputBinding,desk:Area,
    valid:bool,published:bool,pending:Option<CapturedImage>,sas:SasGuard,
}
impl<C:LoginCapture> Context<C> {
    fn matches(&self)->bool {
        if !self.valid||self.capture.output_binding().ok().as_ref()!=Some(&self.output)||desktop()!=self.desk {return false;}
        super::monitors::find(&super::monitors::id_of_device(&self.output.device)).is_some_and(|monitor|
            monitor.device==self.output.device&&monitor.handle.0 as usize==self.output.monitor&&monitor.info.primary
            &&monitor.info.x==self.output.area.left&&monitor.info.y==self.output.area.top
            &&i32::try_from(monitor.info.width).ok()==Some(self.output.area.width)
            &&i32::try_from(monitor.info.height).ok()==Some(self.output.area.height))
    }
    fn probe(&mut self)->io::Result<()> {
        self.probe_with(Self::matches)
    }
    fn probe_with(&mut self,matches:impl Fn(&Self)->bool)->io::Result<()> {
        if !matches(self){self.valid=false;self.pending=None;return Err(expired());}
        // Detect a lost/recreated duplication even when name, bounds and size
        // were reused. Unchanged/Paused cannot erase an unsent valid sample.
        let frame=match self.capture.capture(false) {Ok(frame)=>frame,Err(error)=>{self.valid=false;self.pending=None;return Err(error);}};
        if !matches(self){self.valid=false;self.pending=None;return Err(expired());}
        if let SourceFrame::Frame(image)=frame {
            if image.size()!=self.output.size {self.valid=false;self.pending=None;return Err(expired());}
            self.pending=Some(image);
        }
        Ok(())
    }
    fn capture_with(&mut self,keyframe:bool,matches:impl Fn(&Self)->bool)->io::Result<SourceFrame> {
        self.probe_with(&matches)?;
        if let Some(image)=self.pending.take(){self.published=true;return Ok(SourceFrame::Frame(image));}
        let frame=self.capture.capture(keyframe)?;
        if !matches(self){self.valid=false;self.pending=None;return Err(expired());}
        if let SourceFrame::Frame(image)=&frame {
            if image.size()!=self.output.size {self.valid=false;self.pending=None;return Err(expired());}
            self.published=true;
        }
        Ok(frame)
    }
}
struct LoginSource {context:Arc<Mutex<Context>>,active:bool,size:FrameSize}
impl ScreenSource for LoginSource {
    fn capture_grant(&self)->Option<CaptureGrant> {self.context.lock().ok().and_then(|c|c.capture.capture_grant())}
    fn frame_limits(&self)->FrameLimits {self.context.lock().map(|c|c.capture.frame_limits()).unwrap_or(FrameLimits::LEGACY)}
    fn reference_generation(&self)->u64 {self.context.lock().map_or(0,|c|c.capture.reference_generation())}
    fn size(&self)->FrameSize {self.size}
    fn set_capture_active(&mut self,active:bool)->io::Result<()> {self.active=active;Ok(())}
    fn capture(&mut self,_frame:u64,keyframe:bool)->io::Result<SourceFrame> {
        if !self.active{return Ok(SourceFrame::Unchanged);}
        let mut context=self.context.lock().map_err(|_|expired())?;
        context.capture_with(keyframe,Context::matches)
    }
}
impl Drop for LoginSource {fn drop(&mut self){if let Ok(mut context)=self.context.lock(){context.valid=false;}}}

fn context()->io::Result<Arc<Mutex<Context>>> {
    INPUT.lock().map_err(|_|expired())?.as_ref().and_then(Weak::upgrade).ok_or_else(expired)
}
pub(crate) fn login_geometry_current()->io::Result<bool> {
    let context=match context(){Ok(context)=>context,Err(_)=>return Ok(false)};
    let mut context=context.lock().map_err(|_|expired())?;
    Ok(context.probe().is_ok())
}
/// The no-grant entrypoint is deliberately fail-closed during caller migration.
pub(crate) fn open_login_source()->io::Result<Box<dyn ScreenSource>> {
    Err(io::Error::new(io::ErrorKind::PermissionDenied,"login capture requires prepared parent grant"))
}
struct PreparedLogin {
    capture:PreparedDxgi,
    key:SourceKey,
    admission:Arc<SessionAdmission>,
    device:String,
    monitor:usize,
    area:Area,
    desk:Area,
    selected:Mutex<Option<(CaptureGeometry,HostOutputPlan)>>,
}
impl PreparedLogin {
    fn matches(&self)->bool {
        desktop()==self.desk && super::monitors::find(&super::monitors::id_of_device(&self.device)).is_some_and(|monitor|
            monitor.device==self.device && monitor.handle.0 as usize==self.monitor && monitor.info.primary
            && monitor.info.x==self.area.left && monitor.info.y==self.area.top
            && i32::try_from(monitor.info.width).ok()==Some(self.area.width)
            && i32::try_from(monitor.info.height).ok()==Some(self.area.height))
    }
}
pub(crate) fn prepare_login_source(admission:Arc<SessionAdmission>)->io::Result<Box<dyn PreparedSource>> {
    super::monitors::ensure_dpi_aware();super::mf_common::ensure_com();
    let monitor=super::monitors::enumerate().into_iter().find(|monitor|monitor.info.primary)
        .ok_or_else(||io::Error::new(io::ErrorKind::NotFound,"login monitor unavailable"))?;
    let capture=PreparedDxgi::prepare(&monitor.device)?;
    if capture.monitor()!=monitor.handle.0 as usize {return Err(expired());}
    let key=SourceKey::new(admission.session(),&format!("login:monitor:{}:{}",monitor.info.id,monitor.handle.0 as usize))?;
    let area=Area {left:monitor.info.x,top:monitor.info.y,
        width:i32::try_from(monitor.info.width).map_err(|_|expired())?,
        height:i32::try_from(monitor.info.height).map_err(|_|expired())?};
    let desk=desktop();
    if area.width<=0||area.height<=0||desk.width<=0||desk.height<=0 {return Err(expired());}
    Ok(Box::new(PreparedLogin {capture,key,admission,device:monitor.device,monitor:monitor.handle.0 as usize,
        area,desk,selected:Mutex::new(None)}))
}
impl PreparedSource for PreparedLogin {
    fn source_key(&self)->&SourceKey {&self.key}
    fn description(&self)->PreparedSourceInfo {
        let geometry=self.capture.geometry().ok();
        PreparedSourceInfo {native_size:geometry.map(|g|g.native_displayed()),capture_limits:geometry.map_or(FrameLimits::RESOURCE,|g|g.limits),
            supports_regions:true,state:SessionState::Starting,generation:CaptureGeneration::new(1)}
    }
    fn capture_plan(&self,output:&HostOutputPlan)->io::Result<Option<CapturePlan>> {
        if !self.matches() {return Err(expired());}
        let geometry=self.capture.geometry()?;
        let plan=geometry.plan(self.key.clone(),CaptureGeneration::new(1),output)?;
        let mut selected=self.selected.lock().map_err(|_|expired())?;
        if selected.as_ref().is_some_and(|(_,old)|old.contract()!=output.contract()
            || old.size()!=output.size() || old.codec()!=output.codec()) {
            return Err(io::Error::other("login capture output plan already selected"));
        }
        *selected=Some((geometry,output.clone()));Ok(Some(plan))
    }
    fn activate(self:Box<Self>,grant:CaptureGrant)->Result<Box<dyn ScreenSource>,SourceRefusal> {
        let refuse=|error:io::Error|SourceRefusal::new(&error.to_string());
        if !self.matches() {return Err(refuse(expired()));}
        self.description().check_grant(&self.key,&grant).map_err(refuse)?;
        let (geometry,output)=self.selected.lock().map_err(|_|refuse(expired()))?.take()
            .ok_or_else(||SourceRefusal::new("login output plan not prepared"))?;
        let current=self.capture.geometry().map_err(refuse)?;
        if current.native!=geometry.native || current.kind!=geometry.kind {return Err(refuse(expired()));}
        let Self {capture,key,admission,desk,device,monitor,area,..}=*self;
        let budget=CaptureBudget::activate(admission,key,current,output,grant).map_err(refuse)?;
        let capture=DxgiCapture::open(capture,budget).map_err(refuse)?;
        let output=capture.output_binding().map_err(refuse)?;
        if !output.attached||output.device!=device||output.monitor!=monitor||output.area!=area {return Err(refuse(expired()));}
        let size=capture.size();
        let context=Arc::new(Mutex::new(Context {capture,output,desk,valid:true,published:false,pending:None,sas:SasGuard::new()}));
        if !context.lock().map_err(|_|refuse(expired()))?.matches() {return Err(refuse(expired()));}
        *INPUT.lock().map_err(|_|refuse(expired()))?=Some(Arc::downgrade(&context));
        Ok(Box::new(LoginSource {context,active:true,size}))
    }
}
pub(crate) fn inject_login_input(events:&[InputEvent])->io::Result<()> {
    let context=context()?;let mut context=context.lock().map_err(|_|expired())?;
    let release_only=events.iter().all(|event|matches!(event,InputEvent::Key {down:false,..}|InputEvent::Button {down:false,..}
        |InputEvent::Touch {phase:TouchPhase::End|TouchPhase::Cancel,..}));
    let releases;
    let events=if release_only {
        // Never resolve a monitor or move the pointer while compensating.
        releases=events.iter().filter_map(|event|match event {
            InputEvent::Key {..}|InputEvent::Button {..}=>Some(event.clone()),
            InputEvent::Touch {id:0,..}=>Some(InputEvent::Button {button:PointerButton::Left,down:false}),
            _=>None,
        }).collect::<Vec<_>>();releases.as_slice()
    } else {
        context.probe()?;
        if !context.published{return Err(io::Error::other("login frame not published"));}
        events
    };
    let area=context.output.area;let desk=context.desk;
    // Preserve HID, UTF-16 and SAS rules on the original desktop rectangle.
    let records=super::input_send::records(events,area,desk,&mut context.sas);
    if records.is_empty(){return Ok(());}
    // SAFETY: initialized records on the helper's already-verified desktop.
    let sent=unsafe {SendInput(records.len() as u32,records.as_ptr(),std::mem::size_of::<INPUT>() as i32)};
    if sent as usize!=records.len(){return Err(io::Error::new(io::ErrorKind::PermissionDenied,"login input rejected by Windows"));}
    Ok(())
}

#[cfg(test)]
#[path="batches_capture_recovery_task_tests.rs"]
mod recovery_tests;
