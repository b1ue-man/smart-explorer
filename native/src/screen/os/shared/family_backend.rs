//! Host-side selection leases. The private OS witness is consumed by one worker;
//! the authenticated peer sees only its own opaque selector and display metadata.
use std::collections::HashMap;
use std::io;
use std::sync::{Mutex,atomic::{AtomicBool,AtomicU64,Ordering}};
use crate::remote_window::{NativeWindowId, WindowAdapter, WindowBinding, WindowIdentity};
use crate::screen::core::family_protocol::{token, selector, FamilyListEntry, FamilyListing, FAMILY_PREFIX, FAMILY_PROTOCOL};
use crate::screen::core::window_list::{SourceId, WindowList};

pub(super) struct PinnedFamily {
    adapter: std::sync::Arc<std::sync::Mutex<Box<dyn WindowAdapter>>>,
    binding: WindowBinding,
    lifetime: String,
}
impl PinnedFamily {
    pub(super) fn open(self, program: &std::path::Path, env: &[(String,String)],
        session: &str, control: bool, wanted: Option<crate::screen::core::pixel_convert::FrameSize>)
        -> Result<super::WorkerSource, crate::screen::core::host_backend::SourceRefusal> {
        let pin = self.adapter.lock().map_err(|_|crate::screen::core::host_backend::SourceRefusal::new("family_pin_store_poisoned"))?
            .export_pin(&self.binding)
            .map_err(|error| crate::screen::core::host_backend::SourceRefusal::new(&error.to_string()))?;
        let opened = super::WorkerSource::open_family(program, env,
            crate::screen::core::worker_family::OpenFamily { pin, lifetime: self.lifetime.clone(),
                session: session.into(), control, wanted })?;
        // The receiver has validated the original marker and opened the family.
        // Only now may its original owner relinquish cleanup responsibility.
        self.adapter.lock().map_err(|_|crate::screen::core::host_backend::SourceRefusal::new("family_pin_store_poisoned"))?
            .transfer_pin(&self.binding);
        Ok(opened)
    }
}

#[derive(Default)]
pub(super) struct FamilyPins {
    owners: HashMap<String, HashMap<String, PinnedFamily>>,
    registered: HashMap<String,HashMap<String,(FamilyListEntry,PinnedFamily,crate::remote_window::WindowPin)>>,
    receipts:HashMap<String,HashMap<String,(FamilyListEntry,crate::remote_window::WindowPin)>>,
}
impl FamilyPins {
    pub(super) fn list(&mut self, owner: &str, raw: WindowList) -> io::Result<FamilyListing> {
        // One X11 connection/namespace per listing, not one server client per
        // window. Its pins stay alive through shared ownership until transfer.
        let adapter=std::sync::Arc::new(std::sync::Mutex::new(crate::remote_window::os::adapter()?));
        if let Some(registered)=self.registered.get_mut(owner) {
            registered.retain(|_,(_,pin,_)|pin.adapter.lock().ok().is_some_and(|mut adapter|adapter.validate(&pin.binding).is_ok()));
        }
        let raw = raw.bounded();
        let mut pinned = Vec::new();
        let mut limitations = raw.limits;
        for mut window in raw.windows {
            let Some(native) = window.id.parse::<u64>().ok().filter(|id| *id != 0) else { continue; };
            let result = (|| {
                let mut platform=adapter.lock().map_err(|_|io::Error::other("family_pin_store_poisoned"))?;
                let binding = platform.bind(NativeWindowId(native))?;
                let root = platform.inspect(&binding)?;
                // Preserve P3's catalog mapping only when the freshly pinned
                // platform app label still matches. It is display metadata;
                // the independent OS marker is the selection proof.
                let app_id=root.app_id.clone().or_else(||(window.app==root.app).then(||window.app_id.clone()).flatten());
                let icon = window.icon.take().filter(|icon| app_id.as_deref() == Some(icon.app_id.as_str()));
                window.title = root.title.clone(); window.app = root.app.clone(); window.app_id = app_id;
                window.icon = icon; window.pid = None; window.focused = root.focused; window.minimized = root.minimized;
                platform.validate(&binding)?;
                let lifetime = token()?;
                let entry = FamilyListEntry { launch_request:None,source: SourceId::Window(format!("{FAMILY_PREFIX}{lifetime}")),
                    identity: WindowIdentity { session: lifetime.clone(), generation: 1 }, window: window.clone() };
                Ok::<_,io::Error>((entry, PinnedFamily { adapter:adapter.clone(), binding, lifetime }))
            })();
            match result {
                Ok(entry) => pinned.push(entry),
                Err(error) => limitations.push(format!("family_pin_unavailable:{native}:{error}")),
            }
        }
        if let Some(registered)=self.registered.get(owner) {
            pinned.retain(|(_,pin)|!registered.values().any(|(_,known,_)|known.binding.native==pin.binding.native));
        }
        loop {
            let mut entries:Vec<_>=self.registered.get(owner).into_iter().flat_map(|pins|pins.values().map(|(entry,_,_)|entry.clone())).collect();
            entries.extend(pinned.iter().map(|(entry,_)|entry.clone()));
            let listing = FamilyListing { version: FAMILY_PROTOCOL,
                entries,limitations: limitations.clone() };
            if crate::screen::core::wire::to_json(&listing)?.len() <= crate::screen::core::wire::MAX_WINDOWS_BYTES {
                let pins = pinned.into_iter().map(|(_,pin)| (pin.lifetime.clone(),pin)).collect();
                self.owners.insert(owner.into(), pins);
                listing.validate()?;
                return Ok(listing);
            }
            if pinned.pop().is_none() { return Err(io::Error::other("family_listing_limits_exceed_frame")); }
            if !limitations.iter().any(|reason| reason == "family_list_frame_limit") {
                limitations.push("family_list_frame_limit".into());
            }
        }
    }
    pub(super) fn take(&mut self, owner: &str, source: &SourceId) -> io::Result<PinnedFamily> {
        let selected = selector(source).ok_or_else(|| io::Error::other("not_a_family_selector"))?;
        if let Some(pin)=self.owners.get_mut(owner).and_then(|pins|pins.remove(selected)) {return Ok(pin);}
        if let Some(registered)=self.registered.get_mut(owner) {
            let request=registered.iter().find(|(_, (entry,_,_))|selector(&entry.source)==Some(selected)).map(|(request,_)|request.clone());
            if let Some((_,pin,_))=request.and_then(|request|registered.remove(&request)) {return Ok(pin);}
        }
        Err(io::Error::new(io::ErrorKind::NotFound,"stale_or_foreign_family_selection"))
    }
    pub(super) fn receipt(&self,owner:&str,request:&str,pin:&crate::remote_window::WindowPin)->io::Result<Option<FamilyListEntry>> {
        match self.receipts.get(owner).and_then(|receipts|receipts.get(request)) {
            Some((entry,previous)) if previous==pin=>Ok(Some(entry.clone())),
            Some(_)=>Err(io::Error::new(io::ErrorKind::InvalidInput,"launch_request_pin_changed")),
            None=>Ok(None),
        }
    }
    pub(super) fn register(&mut self,owner:&str,request:&str,pin:crate::remote_window::WindowPin,raw:WindowList)->io::Result<FamilyListEntry> {
        if request.is_empty()||request.len()>128||request.chars().any(char::is_control) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,"invalid_launch_request_identity"));
        }
        if let Some(entry)=self.receipt(owner,request,&pin)? {return Ok(entry);}
        let mut adapter=crate::remote_window::os::adopt(&pin)?;
        let result=(|| {
            let root=adapter.inspect(&pin.binding)?;
            let metadata=raw.windows.iter().find(|window|window.id==pin.binding.native.0.to_string()&&window.app==root.app);
            let app_id=root.app_id.clone().or_else(||metadata.and_then(|window|window.app_id.clone()));
            let icon=metadata.and_then(|window|window.icon.clone()).filter(|icon|app_id.as_deref()==Some(icon.app_id.as_str()));
            let lifetime=token()?;
            let entry=FamilyListEntry {launch_request:Some(request.into()),source:SourceId::Window(format!("{FAMILY_PREFIX}{lifetime}")),
                identity:WindowIdentity {session:lifetime.clone(),generation:1},window:crate::screen::core::window_list::WindowEntry {
                    id:pin.binding.native.0.to_string(),title:root.title,app:root.app,app_id,icon,pid:None,
                    monitor:None,minimized:root.minimized,focused:root.focused}};
            let mut entries:Vec<_>=self.registered.get(owner).into_iter().flat_map(|pins|pins.values().map(|(entry,_,_)|entry.clone())).collect();
            entries.push(entry.clone());
            FamilyListing {version:FAMILY_PROTOCOL,entries,limitations:Vec::new()}.validate()?;
            Ok::<_,io::Error>((entry,lifetime))
        })();
        let (entry,lifetime)=match result {Ok(value)=>value,Err(error)=>{adapter.transfer_pin(&pin.binding);return Err(error);}};
        let binding=pin.binding.clone();
        self.receipts.entry(owner.into()).or_default().insert(request.into(),(entry.clone(),pin.clone()));
        self.registered.entry(owner.into()).or_default().insert(request.into(),(entry.clone(),PinnedFamily {
            adapter:std::sync::Arc::new(std::sync::Mutex::new(adapter)),binding,lifetime},pin));
        Ok(entry)
    }
}

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
    pub(crate) fn open(open: crate::screen::core::worker_family::OpenFamily,
        backend: std::sync::Arc<dyn crate::screen::core::host_backend::ScreenHostBackend>)
        -> io::Result<(Self, Box<dyn crate::screen::core::host_backend::ScreenSource>)> {
        use crate::remote_window::os::family_source::{FamilyScreenSource, SharedAuthority};
        let root_native=open.pin.binding.native.0;
        let adapter = crate::remote_window::os::adopt(&open.pin)?;
        let authority: SharedAuthority = std::sync::Arc::new(Authority {session:open.session.clone(),control:open.control});
        let mut host = crate::remote_window::HostSession::open_bound_as(open.session, open.pin.binding,
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
        let source = FamilyScreenSource::new(host.clone(), authority.clone(), identity,
            Box::new(move |native| member_backend.open_family_member(native)
                .map_err(|refusal|io::Error::other(refusal.reason))),
            crate::screen::core::frame_budget::FrameLimits::RESOURCE)?;
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
