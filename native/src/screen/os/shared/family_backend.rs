//! Host-side selection leases. The private OS witness is consumed by one worker;
//! the authenticated peer sees only its own opaque selector and display metadata.
use std::collections::HashMap;
use std::io;
use std::sync::{Mutex,atomic::{AtomicBool,AtomicU64,Ordering}};
use crate::remote_window::{NativeWindowId, WindowAdapter, WindowBinding, WindowIdentity};
use crate::screen::core::family_protocol::{token, selector, FamilyListEntry, FamilyListing, FAMILY_PREFIX, FAMILY_PROTOCOL};
use crate::screen::core::window_list::{SourceId, WindowList};

#[path = "family_pins.rs"]
mod pins;
pub(super) use pins::FamilyPins;

struct Authority { session: String, control: bool }
impl crate::remote_window::WindowAuthority for Authority {
    fn may_view(&self, session: &str) -> bool { session == self.session }
    fn may_control(&self, session: &str) -> bool { session == self.session && self.control }
    fn in_list_scope(&self, _app_id: Option<&str>) -> bool { true }
}

pub(crate) struct WorkerFamily {
    host: crate::remote_window::os::family_source::SharedHost,
    authority: crate::remote_window::os::family_source::SharedAuthority,
    backend: std::sync::Arc<dyn crate::screen::core::host_backend::ScreenHostBackend>,
    pressed: Mutex<crate::screen::core::input_model::PressedInputs>,
    release_pending: AtomicBool,
    input_epoch: AtomicU64,
    completed: std::collections::VecDeque<(crate::screen::core::family_protocol::FamilyOperation,
        crate::screen::core::family_protocol::FamilyOperationResult)>,
}

impl WorkerFamily {
    pub(crate) fn prepare(open: crate::screen::core::worker_family::OpenFamily,
        backend: std::sync::Arc<dyn crate::screen::core::host_backend::ScreenHostBackend>,
        admission: std::sync::Arc<crate::screen::core::capture_admission::SessionAdmission>, owner: &str)
        -> io::Result<(Self, Box<dyn crate::screen::core::host_backend::PreparedSource>)> {
        use crate::remote_window::os::family_source::{PreparedFamily, SharedAuthority};
        let root_native=open.pin.binding.native.0;
        let adapter = crate::remote_window::os::adopt(&open.pin)?;
        let authority: SharedAuthority = std::sync::Arc::new(Authority {session:open.session.clone(),control:open.control});
        let mut host = crate::remote_window::HostSession::open_bound_as(open.session.clone(), open.pin.binding,
            adapter, authority.as_ref(), open.lifetime)?;
        let snapshot=host.snapshot(authority.as_ref())?;
        if let Ok(list)=backend.windows() {
            let icon=list.windows.iter().find(|window|window.id==root_native.to_string())
                .and_then(|window|window.icon.clone());
            if let Some(icon)=icon {
                for member in snapshot.members.iter().filter(|member|member.app_id.as_deref()==Some(icon.app_id.as_str())) {
                    let _=host.set_icon(&member.identity,Some(icon.clone()),authority.as_ref());
                }
            }
        }
        let host = std::sync::Arc::new(std::sync::Mutex::new(host));
        let identity = host.lock().map_err(|_|io::Error::other("family_host_poisoned"))?
            .snapshot(authority.as_ref())?.root;
        let member_backend = backend.clone();
        let member_owner = owner.to_owned();
        let member_session = open.session;
        let source = PreparedFamily::new(host.clone(), authority.clone(), identity,
            std::sync::Arc::new(move |native, context| {
                member_backend.prepare_family_member(&member_owner, &member_session, native, context)
                    .map_err(|refusal| io::Error::other(refusal.reason))
            }), admission, crate::screen::core::frame_budget::FrameLimits::RESOURCE)?;
        let input_epoch=host.lock().map_err(|_|io::Error::other("family_host_poisoned"))?.input_epoch();
        Ok((Self {host,authority,backend,pressed:Default::default(),release_pending:AtomicBool::new(false),
            input_epoch:AtomicU64::new(input_epoch),completed:Default::default()}, Box::new(source)))
    }

    pub(crate) fn snapshot(&self) -> io::Result<crate::remote_window::FamilySnapshot> {
        let result=(|| {
            let mut host=self.host.lock().map_err(|_|io::Error::other("family_host_poisoned"))?;
            let snapshot=host.snapshot(self.authority.as_ref())?;
            Ok::<_,io::Error>((snapshot,host.input_epoch()))
        })();
        match result {
            Ok((snapshot,epoch))=>{
                if self.input_epoch.swap(epoch,Ordering::SeqCst)!=epoch || self.release_pending.load(Ordering::SeqCst) {
                    // A failed OS release remains pending, while viewing and
                    // subsequent snapshot polls continue to retry it.
                    let _=self.release();
                }
                Ok(snapshot)
            }
            Err(error)=>{let _=self.release();Err(error)}
        }
    }

    pub(crate) fn operation(&mut self, request: crate::screen::core::family_protocol::FamilyOperation)
        -> io::Result<crate::screen::core::family_protocol::FamilyOperationResult> {
        use crate::screen::core::family_protocol::FamilyOperationResult;
        request.validate()?;
        let current=self.snapshot()?;
        // Never carry held modifiers/buttons through an explicit activation.
        if request.operation==crate::remote_window::WindowOperation::Focus {
            if self.release().is_err() {
                return Ok(FamilyOperationResult {request_id:request.request_id,status:None,
                    reason:Some("family_input_release_pending".into()),snapshot:Some(current)});
            }
        }
        if let Some((previous,result)) = self.completed.iter().find(|(old,_)| old.request_id == request.request_id) {
            if previous != &request { return Err(io::Error::new(io::ErrorKind::InvalidInput,"family_request_id_reused")); }
            let mut current=result.clone();current.snapshot=Some(self.snapshot()?);return Ok(current);
        }
        let mut host = self.host.lock().map_err(|_|io::Error::other("family_host_poisoned"))?;
        let result = host.operate(&request.identity, request.operation, self.authority.as_ref());
        let snapshot = host.snapshot(self.authority.as_ref()).ok();
        let reply = match result {
            Ok(status) => FamilyOperationResult { request_id:request.request_id.clone(), status:Some(status),reason:None,snapshot },
            Err(error) => FamilyOperationResult { request_id:request.request_id.clone(),status:None,reason:Some(error.to_string()),snapshot },
        };
        // Same bounded recent-command horizon as the existing control inbox.
        if self.completed.len() == 64 { self.completed.pop_front(); }
        self.completed.push_back((request,reply.clone()));
        Ok(reply)
    }

    pub(crate) fn input(&mut self, request: crate::screen::core::family_protocol::FamilyInput)
        -> io::Result<crate::screen::core::family_protocol::FamilyInputResult> {
        use crate::screen::core::wire_control::ControlBlock;
        request.validate()?;
        self.snapshot()?;
        if self.release_pending.load(Ordering::SeqCst) {
            if let Err(blocked)=self.release() {
                return Ok(crate::screen::core::family_protocol::FamilyInputResult {
                    blocked:Some(blocked),reason:Some("family_input_release_pending".into())});
            }
        }
        let mut blocked = None;
        let backend = &self.backend;
        let result = self.host.lock().map_err(|_|io::Error::other("family_host_poisoned"))?
            .with_input(&request.target.identity, request.target.snapshot_revision, self.authority.as_ref(), |member| {
                super::super::note_input_target(&SourceId::Window(member.binding.native.0.to_string()));
                backend.inject_family(member,&request.events).map_err(|reason| {blocked=Some(reason);io::Error::other("platform_input_blocked")})
            });
        match result {
            Ok(()) => {
                let mut pressed=self.pressed.lock().map_err(|_|io::Error::other("family_input_state_poisoned"))?;
                for event in &request.events {pressed.observe(event);}
                Ok(crate::screen::core::family_protocol::FamilyInputResult {blocked:None,reason:None})
            }
            Err(error) => {
                // SendInput can fail after a partial native batch. Retain every
                // possibly pressed down event until release actually succeeds.
                if blocked.is_some() {
                    let mut pressed=self.pressed.lock().map_err(|_|io::Error::other("family_input_state_poisoned"))?;
                    for event in &request.events {
                        if matches!(event,crate::screen::core::input_model::InputEvent::Key {down:true,..}
                            |crate::screen::core::input_model::InputEvent::Button {down:true,..}
                            |crate::screen::core::input_model::InputEvent::Touch {phase:crate::screen::core::input_model::TouchPhase::Start|crate::screen::core::input_model::TouchPhase::Move,..}) {
                            pressed.observe(event);
                        }
                    }
                }
                // Snapshot/focus failures also release previously held input;
                // a target's current permission is never required for its ups.
                let _=self.release();
                Ok(crate::screen::core::family_protocol::FamilyInputResult {
                    blocked:Some(blocked.unwrap_or(ControlBlock::ControlUnavailable)),reason:Some(error.to_string()) })
            },
        }
    }

    pub(crate) fn release(&self) -> Result<(),crate::screen::core::wire_control::ControlBlock> {
        use crate::screen::core::wire_control::ControlBlock;
        let mut pressed=self.pressed.lock().map_err(|_|ControlBlock::ControlUnavailable)?;
        let mut pending=pressed.clone();
        let released = pending.release_all();
        if released.is_empty() {self.release_pending.store(false,Ordering::SeqCst);return Ok(());}
        self.release_pending.store(true,Ordering::SeqCst);
        // Releases contain no position/text/action. Their delivery must remain
        // possible after the original app window vanished, without focusing one.
        self.backend.release_input(&released)?;
        *pressed=pending;self.release_pending.store(false,Ordering::SeqCst);Ok(())
    }
}

impl Drop for WorkerFamily { fn drop(&mut self) { let _ = self.release(); } }
