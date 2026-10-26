//! An immutable output/desktop binding inside the verified login helper.
use std::io;
use std::sync::{Arc,Mutex,Weak};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput,INPUT};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics,SM_CXVIRTUALSCREEN,SM_CYVIRTUALSCREEN,SM_XVIRTUALSCREEN,SM_YVIRTUALSCREEN};
use super::capture_dxgi::{DxgiCapture,OutputBinding};
use super::capture_source::Grabber;
use super::input_send::Area;
use super::input_keys::SasGuard;
use crate::screen::core::host_backend::{ScreenSource,SourceFrame};
use crate::screen::core::input_model::{InputEvent,PointerButton,TouchPhase};
use crate::screen::core::pixel_convert::{FrameSize,BgraImage};

static INPUT:Mutex<Option<Weak<Mutex<Context>>>>=Mutex::new(None);
fn expired()->io::Error {io::Error::new(io::ErrorKind::PermissionDenied,"login capture geometry changed; start a new login connection")}
fn desktop()->Area {
    // SAFETY: read-only metrics on the helper's verified input desktop.
    unsafe {Area {left:GetSystemMetrics(SM_XVIRTUALSCREEN),top:GetSystemMetrics(SM_YVIRTUALSCREEN),
        width:GetSystemMetrics(SM_CXVIRTUALSCREEN),height:GetSystemMetrics(SM_CYVIRTUALSCREEN)}}
}
struct Context {
    capture:DxgiCapture,output:OutputBinding,desk:Area,
    valid:bool,published:bool,pending:Option<BgraImage>,sas:SasGuard,
}
impl Context {
    fn matches(&self)->bool {
        if !self.valid||self.capture.output_binding().ok().as_ref()!=Some(&self.output)||desktop()!=self.desk {return false;}
        super::monitors::find(&super::monitors::id_of_device(&self.output.device)).is_some_and(|monitor|
            monitor.device==self.output.device&&monitor.handle.0 as usize==self.output.monitor&&monitor.info.primary
            &&monitor.info.x==self.output.area.left&&monitor.info.y==self.output.area.top
            &&i32::try_from(monitor.info.width).ok()==Some(self.output.area.width)
            &&i32::try_from(monitor.info.height).ok()==Some(self.output.area.height))
    }
    fn probe(&mut self)->io::Result<()> {
        if !self.matches(){self.valid=false;return Err(expired());}
        // Detect a lost/recreated duplication even when name, bounds and size
        // were reused. Keep the latest sampled pixels for the next Capture.
        let frame=match self.capture.capture(false) {Ok(frame)=>frame,Err(error)=>{self.valid=false;return Err(error);}};
        if !self.matches(){self.valid=false;self.pending=None;return Err(expired());}
        if let SourceFrame::Frame(image)=frame {
            if image.size()!=self.output.size {self.valid=false;return Err(expired());}
            self.pending=Some(image);
        }
        Ok(())
    }
}
struct LoginSource {context:Arc<Mutex<Context>>,active:bool,size:FrameSize}
impl ScreenSource for LoginSource {
    fn size(&self)->FrameSize {self.size}
    fn set_capture_active(&mut self,active:bool)->io::Result<()> {self.active=active;Ok(())}
    fn capture(&mut self,_frame:u64,keyframe:bool)->io::Result<SourceFrame> {
        if !self.active{return Ok(SourceFrame::Unchanged);}
        let mut context=self.context.lock().map_err(|_|expired())?;
        context.probe()?;
        if let Some(image)=context.pending.take(){context.published=true;return Ok(SourceFrame::Frame(image));}
        let frame=context.capture.capture(keyframe)?;
        if !context.matches(){context.valid=false;return Err(expired());}
        if matches!(frame,SourceFrame::Frame(_)){context.published=true;}
        Ok(frame)
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
pub(crate) fn open_login_source()->io::Result<Box<dyn ScreenSource>> {
    super::monitors::ensure_dpi_aware();
    let monitor=super::monitors::enumerate().into_iter().find(|monitor|monitor.info.primary)
        .ok_or_else(||io::Error::new(io::ErrorKind::NotFound,"login monitor unavailable"))?;
    let capture=DxgiCapture::open(&monitor.device).map_err(io::Error::other)?;
    let output=capture.output_binding()?;let desk=desktop();
    if !output.attached||output.device!=monitor.device||output.monitor!=monitor.handle.0 as usize
        ||output.area.width<=0||output.area.height<=0||desk.width<=0||desk.height<=0{return Err(expired());}
    let size=capture.size();
    let context=Arc::new(Mutex::new(Context {capture,output,desk,valid:true,published:false,pending:None,sas:SasGuard::new()}));
    if !context.lock().map_err(|_|expired())?.matches(){return Err(expired());}
    *INPUT.lock().map_err(|_|expired())?=Some(Arc::downgrade(&context));
    Ok(Box::new(LoginSource {context,active:true,size}))
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
