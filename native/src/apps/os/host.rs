//! Sole launch boundary. Network callers provide an E4 admission closure here;
//! holding its guard through `dispatch` prevents a policy/launch TOCTOU gap.
use std::{io, path::{Path,PathBuf}, sync::{Mutex,Arc}};
use sha2::{Digest, Sha256};
use crate::apps::core::model::*;
use super::shared::{journal::Journal, platform::{HostPlatform, PlatformCatalog}};
use crate::screen::core::app_icon::{IconRef,IconReply};

#[cfg(test)]
#[path = "batches_launch_task_tests.rs"]
mod batches_launch_task_tests;

#[derive(serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerRequestBinding {version:u8,owner:String,request:LaunchRequest}

pub(crate) struct CatalogIcons {
    pub references: fn(&mut [CatalogApp]),
    pub content: fn(&CatalogApp,&IconRef)->io::Result<Option<IconReply>>,
}
/// Main supplies the actual E4 grant end, alongside its policy/revocation guard.
/// `None` means the authoritative grant has no deadline, not "not checked".
pub(crate) struct LaunchAdmission<G> {
    pub guard:G,
    pub expires_at_unix_ms:Option<i64>,
    pub valid:Option<Arc<dyn Fn()->bool+Send+Sync>>,
}

pub(crate) struct AppHost<P: HostPlatform = super::platform::Platform> {
    device: DeviceId,
    platform: P,
    journal: Journal,
    serial: Mutex<()>,
    icons: Option<CatalogIcons>,
    ownership_root:PathBuf,
    windows:Mutex<super::shared::window_correlation::Correlations>,
}
impl AppHost<super::platform::Platform> {
    pub(crate) fn for_current_user(device:DeviceId,root:&Path,icons:Option<CatalogIcons>)->io::Result<Self> {
        let host=Self::new(device,super::platform::Platform::default(),root)?;
        Ok(if let Some(icons)=icons {host.with_icons(icons)} else {host})
    }
}
pub(crate) fn local_consent_supported()->bool {cfg!(target_os="android")}
pub(crate) fn install_platform_hooks(result:fn(LaunchRequest,String,super::shared::platform::DispatchReceipt),consent:fn(LaunchRequest))->io::Result<()> {
    #[cfg(target_os="android")]
    {super::platform::bridge::set_result_sink(result)?;super::platform::bridge::set_consent_sink(consent)?;}
    #[cfg(not(target_os="android"))]
    let _=(result,consent);
    Ok(())
}
pub(crate) fn end_platform_consent(request:&LaunchRequest,reason:&str) {
    #[cfg(target_os="android")]
    super::platform::bridge::end_local_consent(request,reason);
    #[cfg(not(target_os="android"))]
    let _=(request,reason);
}
pub(crate) fn claim_local_admission(dispatch:&str,request:&str)->io::Result<(i64,u64)> {
    #[cfg(target_os="android")]
    {let permit=super::platform::bridge::claim_admission(dispatch,request)?;return Ok((permit.expires_at_unix_ms,permit.valid_for_ms));}
    #[cfg(not(target_os="android"))]
    {let _=(dispatch,request);Err(io::Error::new(io::ErrorKind::Unsupported,"local platform admission is unavailable"))}
}
pub(crate) fn finish_local_icon(challenge:&str,reply:Option<IconReply>)->io::Result<()> {
    #[cfg(target_os="android")]
    {return super::platform::bridge::finish_icon(challenge,reply);}
    #[cfg(not(target_os="android"))]
    {let _=(challenge,reply);Err(io::Error::new(io::ErrorKind::Unsupported,"local icon bridge is unavailable"))}
}
impl<P: HostPlatform> AppHost<P> {
    pub(crate) fn new(device: DeviceId, platform: P, ledger_dir: &Path) -> io::Result<Self> {
        Ok(Self { device, platform, journal: Journal::new(ledger_dir)?, serial: Mutex::new(()), icons:None,ownership_root:ledger_dir.to_owned(),windows:Mutex::new(Default::default()) })
    }
    /// A stable request ID has one exact authenticated peer owner for its whole
    /// durable lifetime. A caller cannot obtain an unowned or foreign receipt.
    pub(crate) fn bind_peer_owner(&self,id:&RequestId,owner:&str,claim:bool)->io::Result<bool> {
        use std::io::{Read,Write};
        if owner.len()!=64||!owner.bytes().all(|b|b.is_ascii_hexdigit()) {return Err(invalid("invalid peer ownership digest"));}
        let path=self.ownership_root.join(format!("{:x}.peer",Sha256::digest(id.as_str().as_bytes())));
        let read=||->io::Result<Option<bool>> {
            let mut file=match std::fs::File::open(&path) {Ok(f)=>f,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(e)};
            if !file.metadata()?.is_file()||std::fs::symlink_metadata(&path)?.file_type().is_symlink() {return Err(invalid("invalid peer ownership file"));}
            let mut bytes=Vec::new();std::io::Read::by_ref(&mut file).take(65).read_to_end(&mut bytes)?;Ok(Some(bytes==owner.as_bytes()))
        };
        if let Some(same)=read()? {return Ok(same);}
        if !claim {return Ok(false);}
        let mut file=tempfile::NamedTempFile::new_in(&self.ownership_root)?;file.write_all(owner.as_bytes())?;file.as_file().sync_all()?;
        if super::platform::publish_record(file,&path,false)? {super::platform::sync_journal(&self.ownership_root)?;return Ok(true);}
        Ok(read()?.unwrap_or(false))
    }
    pub(crate) fn bind_peer_request(&self,request:&LaunchRequest,owner:&str,claim:bool)->io::Result<bool> {
        use std::io::{Read,Write};
        if owner.len()!=64||!owner.bytes().all(|b|b.is_ascii_hexdigit()) {return Err(invalid("invalid peer ownership digest"));}
        let hash=format!("{:x}",Sha256::digest(request.request_id.as_str().as_bytes()));
        let path=self.ownership_root.join(format!("{hash}.peer-request-v1"));
        let read=||->io::Result<Option<bool>> {
            let file=match std::fs::File::open(&path) {Ok(file)=>file,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(e)};
            if !file.metadata()?.is_file()||std::fs::symlink_metadata(&path)?.file_type().is_symlink() {return Err(invalid("invalid peer request binding file"));}
            let mut bytes=Vec::new();file.take(32769).read_to_end(&mut bytes)?;
            if bytes.len()>32768 {return Err(invalid("peer request binding exceeds byte budget"));}
            let bound:PeerRequestBinding=serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if bound.version!=1 {return Err(invalid("unsupported peer request binding version"));}
            Ok(Some(bound.owner==owner&&bound.request==*request))
        };
        let existing=read()?;
        if existing==Some(false) {return Ok(false);}
        if existing.is_none() {
            let legacy_owner=std::fs::symlink_metadata(self.ownership_root.join(format!("{hash}.peer"))).is_ok();
            let legacy_body=self.journal.bound_request(&request.request_id)?;
            if legacy_owner {
                if !self.bind_peer_owner(&request.request_id,owner,false)? {return Ok(false);}
                if legacy_body.as_ref()!=Some(request) {return Err(invalid("legacy app request owner has no matching durable body"));}
            } else if legacy_body.is_some()||!claim {return Ok(false);}
            // Owner and complete body have one atomic no-replace publication.
            // A crash cannot leave an ID owned but accepting a changed request.
            let bytes=serde_json::to_vec(&PeerRequestBinding {version:1,owner:owner.into(),request:request.clone()}).map_err(io::Error::other)?;
            if bytes.len()>32768 {return Err(invalid("peer request binding exceeds byte budget"));}
            let mut stage=tempfile::NamedTempFile::new_in(&self.ownership_root)?;stage.write_all(&bytes)?;stage.as_file().sync_all()?;
            if super::platform::publish_record(stage,&path,false)? {super::platform::sync_journal(&self.ownership_root)?;}
            if read()?!=Some(true) {return Ok(false);}
        }
        // Maintain the existing local bridge/host journal readers only after
        // the atomic owner/body record has established their authority.
        if !self.bind_peer_owner(&request.request_id,owner,true)? {return Ok(false);}
        self.journal.bind(request)
    }
    pub(crate) fn with_icons(mut self,icons:CatalogIcons)->Self {self.icons=Some(icons);self}
    fn snapshot(&self) -> io::Result<(CatalogSnapshot, PlatformCatalog<P::Target>)> {
        let mut native = self.platform.snapshot()?;
        native.apps.sort_by(|a,b| a.app.app_id.cmp(&b.app.app_id));
        if native.apps.windows(2).any(|a| a[0].app.app_id == a[1].app.app_id) {
            return Err(io::Error::other("ambiguous installed application identity"));
        }
        let mut hash = Sha256::new();
        // Lock/focus changes require fresh admission, but are not a new login or
        // installed catalog. A local unlock can continue the SAME request ID.
        hash.update(serde_json::to_vec(&(&self.device,&native.session.user_id,&native.session.session_id)).map_err(io::Error::other)?);
        for app in &native.apps {
            hash.update(serde_json::to_vec(&(&app.app.app_id,&app.app.name,&app.fingerprint,app.app.can_launch)).map_err(io::Error::other)?);
        }
        let revision=format!("{:x}",hash.finalize());
        let authorization_scope=SessionId::try_from(format!("app-catalog:{revision}"))?;
        let mut catalog = CatalogSnapshot { version: 1, device_id: self.device.clone(),
            session: native.session.clone(),revision,authorization_scope,
            apps: native.apps.iter().map(|a|a.app.clone()).collect(), limits: native.limits.clone() };
        if let Some(icons)=&self.icons {(icons.references)(&mut catalog.apps);}
        for app in &mut catalog.apps {
            if app.icon.as_ref().is_some_and(|icon|icon.validate().is_err()||icon.app_id!=app.app_id.as_str()) {app.icon=None;}
        }
        Ok((catalog,native))
    }
    pub(crate) fn catalog(&self) -> io::Result<CatalogSnapshot> { self.snapshot().map(|s|s.0) }
    /// Main authorizes catalog/icon disclosure before invoking this local read.
    pub(crate) fn icon(&self,id:&AppId,expected:&IconRef)->io::Result<Option<IconReply>> {
        let catalog=self.catalog()?;
        let app=catalog.apps.iter().find(|a|&a.app_id==id && a.icon.as_ref()==Some(expected))
            .ok_or_else(||invalid("icon no longer belongs to current installed catalog"))?;
        let reply=if let Some(icons)=&self.icons {(icons.content)(app,expected)?} else {self.platform.icon(app)?};
        if let Some(reply)=&reply {reply.bytes_for(expected)?;}
        Ok(reply)
    }

    fn receipt_for(request: &LaunchRequest, state: LaunchState, reason: impl Into<String>) -> LaunchReceipt {
        let mut hash=Sha256::new();
        for part in [request.request_id.as_str(),request.target_device_id.as_str(),request.app_id.as_str(),
            request.user_id.as_str(),request.session_id.as_str(),&request.catalog_revision] {
            hash.update((part.len() as u64).to_le_bytes()); hash.update(part.as_bytes());
        }
        LaunchReceipt { request: request.clone(), launch_id: format!("launch:{:x}",hash.finalize()),
            dispatch_id:String::new(),state, reason: reason.into(), pid: None, window: None }
    }
    fn new_dispatch(request:&LaunchRequest,reason:&str)->io::Result<LaunchReceipt> {
        let mut bytes=[0u8;32];getrandom::getrandom(&mut bytes).map_err(|_|io::Error::other("launch dispatch ID source unavailable"))?;
        let mut receipt=Self::receipt_for(request,LaunchState::OutcomeUnknown,reason);
        receipt.dispatch_id=bytes.iter().map(|b|format!("{b:02x}")).collect();Ok(receipt)
    }
    fn facts(&self, request: &LaunchRequest, catalog: &CatalogSnapshot) -> Result<LaunchFacts,String> {
        if request.catalog_revision.len()!=64 || !request.catalog_revision.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {
            return Err("invalid_catalog_revision".into());
        }
        if request.target_device_id != self.device { return Err("target_device_mismatch".into()); }
        if request.user_id != catalog.session.user_id || request.session_id != catalog.session.session_id {
            return Err("host_user_or_login_session_changed".into());
        }
        if request.catalog_revision != catalog.revision { return Err("host_catalog_changed".into()); }
        let app = catalog.apps.iter().find(|a| a.app_id == request.app_id)
            .ok_or_else(|| "app_not_in_current_host_catalog".to_string())?;
        if !catalog.session.interactive { return Err("host_has_no_authorized_interactive_session".into()); }
        Ok(LaunchFacts { request:request.clone(),catalog:catalog.clone(),app:app.clone(),authorization_scope:catalog.authorization_scope.clone() })
    }
    /// Repetition returns the durable receipt; changed contents under an ID are
    /// refused. `G` may be an E4 revocation/policy read guard. It is not serialized.
    pub(crate) fn launch<G>(&self, request: &LaunchRequest,
        authorize: impl FnOnce(&LaunchFacts) -> Result<LaunchAdmission<G>,LaunchDenial>) -> io::Result<LaunchReceipt> {
        self.launch_inner(request, false, authorize)
    }
    /// Local-UI continuation only. Main must retain the original authenticated
    /// peer context and run E4 again; never expose this flag on the remote wire.
    pub(crate) fn resume_after_local_consent<G>(&self, request: &LaunchRequest,
        authorize: impl FnOnce(&LaunchFacts) -> Result<LaunchAdmission<G>,LaunchDenial>) -> io::Result<LaunchReceipt> {
        self.launch_inner(request, true, authorize)
    }
    fn launch_inner<G>(&self, request: &LaunchRequest, local_consent: bool,
        authorize: impl FnOnce(&LaunchFacts) -> Result<LaunchAdmission<G>,LaunchDenial>) -> io::Result<LaunchReceipt> {
        let _serial = self.serial.lock().map_err(|_|io::Error::other("launch serialization poisoned"))?;
        if !self.journal.bind(request)? {return Ok(Self::receipt_for(request,LaunchState::Denied,"request_id_reused_with_different_request"));}
        let previous = self.journal.read(&request.request_id)?;
        if previous.as_ref().is_some_and(|p|p.request != *request) {
            return Ok(Self::receipt_for(request,LaunchState::Denied,"request_id_reused_with_different_request"));
        }
        let (catalog,native) = self.snapshot()?;
        let facts = match self.facts(request,&catalog) {
            Ok(facts) => facts,
            Err(reason) => return self.denial(request,LaunchState::Denied,reason,previous.is_some()),
        };
        let admission = match authorize(&facts) {
            Ok(guard) => guard,
            Err(denial) => { let (state,reason)=denial.parts(); return self.denial(request,state,reason,previous.is_some()); }
        };
        if super::shared::platform::expired(admission.expires_at_unix_ms) {
            return self.denial(request,LaunchState::Denied,"app_launch_grant_expired".into(),previous.is_some());
        }
        let deadline=admission.expires_at_unix_ms;
        let valid=admission.valid;
        let _admission_guard=admission.guard;
        if let Some(receipt) = previous {
            if !local_consent || receipt.state != LaunchState::ConsentRequired { return Ok(receipt); }
            // Across processes only the creator of a new continuation claim may
            // dispatch. The original request itself remains permanently claimed.
            return self.continue_claimed(request,&native,deadline,valid);
        }
        let receipt = Self::new_dispatch(request,"admitted_os_result_not_yet_confirmed")?;
        if !self.journal.claim(&receipt)? {
            return self.claimed_receipt(request);
        }
        self.dispatch_claimed(request,&native,receipt,deadline,valid)
    }
    fn claimed_receipt(&self,request:&LaunchRequest)->io::Result<LaunchReceipt> {
        let receipt=self.journal.read(&request.request_id)?.ok_or_else(||io::Error::other("launch claim disappeared"))?;
        if receipt.request!=*request {return Ok(Self::receipt_for(request,LaunchState::Denied,"request_id_reused_with_different_request"));}
        Ok(receipt)
    }
    fn denial(&self,request:&LaunchRequest,state:LaunchState,reason:String,already_bound:bool)->io::Result<LaunchReceipt> {
        let receipt=Self::receipt_for(request,state,reason);
        if !already_bound && !self.journal.claim(&receipt)? {return self.claimed_receipt(request);}
        if already_bound && self.journal.read(&request.request_id)?.is_some_and(|old|old.state==LaunchState::ConsentRequired)
            && matches!(receipt.state,LaunchState::Denied|LaunchState::Unsupported) {self.journal.update(&receipt)?;}
        Ok(receipt)
    }
    fn continue_claimed(&self, request: &LaunchRequest, native: &PlatformCatalog<P::Target>,deadline:Option<i64>,valid:Option<Arc<dyn Fn()->bool+Send+Sync>>) -> io::Result<LaunchReceipt> {
        // A second durable ID prevents two local continuations racing. It is
        // internal and cannot collide with the public request-key namespace.
        let continuation_dir = self.journal_continuation(request)?;
        if !continuation_dir { return self.journal.read(&request.request_id)?.ok_or_else(||io::Error::other("missing launch receipt")); }
        let receipt = Self::new_dispatch(request,"local_consent_admitted_os_result_not_yet_confirmed")?;
        self.journal.update(&receipt)?;
        let receipt=self.dispatch_claimed(request,native,receipt,deadline,valid)?;
        if receipt.state==LaunchState::ConsentRequired {self.journal.release_unstarted_continuation(&request.request_id)?;}
        Ok(receipt)
    }
    fn journal_continuation(&self, request: &LaunchRequest) -> io::Result<bool> {
        self.journal.claim_continuation(&request.request_id)
    }
    fn dispatch_claimed(&self, request: &LaunchRequest, native: &PlatformCatalog<P::Target>, mut receipt: LaunchReceipt,deadline:Option<i64>,valid:Option<Arc<dyn Fn()->bool+Send+Sync>>) -> io::Result<LaunchReceipt> {
        let installed = native.apps.iter().find(|a| a.app.app_id == request.app_id)
            .ok_or_else(||io::Error::other("catalog entry vanished before dispatch"))?;
        if valid.as_ref().is_some_and(|check|!check()) {
            receipt.state=LaunchState::Denied;receipt.reason="app_admission_revoked_or_cancelled".into();
        } else if !installed.app.can_launch {
            receipt.state=LaunchState::Unsupported;
            receipt.reason=installed.app.unavailable_reason.clone().unwrap_or_else(||"app_start_unavailable".into());
        } else {
            let known=self.windows.lock().map_err(|_|io::Error::other("app window registry unavailable"))?
                .activate_known(request,deadline,&valid,||self.snapshot().is_ok_and(|(catalog,_)|
                    catalog.session.unlocked&&self.facts(request,&catalog).is_ok()));
            match known {
                Ok(Some(focused))=>{
                    receipt.state=LaunchState::StartedWindowPending;
                    receipt.reason=if focused {"known_instance_activation_confirmed_p4_registration_pending"} else {"known_instance_available_activation_not_confirmed_p4_registration_pending"}.into();
                    self.journal.update(&receipt)?;return Ok(receipt);
                },
                Err(error)=>{
                    if error.to_string()=="app_activation_admission_ended" {receipt.state=LaunchState::Denied;receipt.reason="app_activation_admission_ended".into();}
                    else {receipt.reason=format!("known_instance_activation_result_unknown: {error}");}
                    self.journal.update(&receipt)?;return Ok(receipt);
                },
                Ok(None)=>{},
            }
            #[cfg(target_os="android")]
            let _android_admission=super::platform::bridge::hold_admission(&receipt.dispatch_id,request.request_id.as_str(),deadline,valid)?;
            match self.platform.dispatch(&installed.target,request,&native.session,deadline,&receipt.dispatch_id) {
                Ok(result) => { receipt.state=result.state; receipt.reason=result.reason; receipt.pid=result.pid; }
                // An ambiguous OS/bridge error cannot prove that nothing started.
                Err(error) => { receipt.reason=format!("os_dispatch_result_unknown: {error}"); }
            }
            self.windows.lock().map_err(|_|io::Error::other("app window registry unavailable"))?.take_started(request,&receipt.dispatch_id);
        }
        self.journal.update(&receipt)?;
        Ok(receipt)
    }
    /// Read-only recovery after timeout/lost response. Main checks permission
    /// before returning this local receipt to a remote peer.
    pub(crate) fn receipt(&self, request: &LaunchRequest) -> io::Result<Option<LaunchReceipt>> {
        if request.target_device_id!=self.device {return Err(invalid("receipt target device identity changed"));}
        let receipt=self.journal.read(&request.request_id)?;
        if receipt.as_ref().is_some_and(|p|p.request!=*request) { return Err(invalid("request ID contents differ")); }
        Ok(receipt)
    }
    /// Observation-only continuation/recovery. The caller supplies the exact
    /// authenticated P4 owner; registration gives no ScreenView/Control grant.
    pub(crate) fn correlate_window(&self,request:&LaunchRequest,
        register:impl FnOnce(crate::remote_window::WindowPin)->io::Result<crate::screen::core::family_protocol::FamilyListEntry>)
        ->io::Result<Option<LaunchReceipt>> {
        let Some(receipt)=self.receipt(request)? else {return Ok(None);};
        if !matches!(receipt.state,LaunchState::OutcomeUnknown|LaunchState::StartedWindowPending|LaunchState::Started|LaunchState::Activated) {return Ok(Some(receipt));}
        let session=self.platform.session();
        if !session.is_ok_and(|session|session.user_id==request.user_id&&session.session_id==request.session_id&&session.interactive) {
            return self.window_pending(request,"original_app_logon_or_user_ended").map(Some);
        }
        let result=self.windows.lock().map_err(|_|io::Error::other("app window registry unavailable"))?.confirm(request,register);
        let confirmed=match result {
            Ok(Some(confirmed))=>confirmed,
            Ok(None)=>return self.window_pending(request,"original_process_witness_unavailable; no_window_or_projection_selection_invented").map(Some),
            Err(error)=>return self.window_pending(request,&format!("window_not_confirmed: {error}")).map(Some),
        };
        let entry=&confirmed.entry;
        if entry.launch_request.as_deref()!=Some(request.request_id.as_str())
            ||crate::screen::core::family_protocol::selector(&entry.source)!=Some(entry.identity.session.as_str())
            ||entry.identity.generation!=1 {
            return self.window_pending(request,"p4_registration_request_or_identity_mismatch").map(Some);
        }
        let crate::screen::core::window_list::SourceId::Window(window_id)=&entry.source else {return self.window_pending(request,"p4_registration_is_not_a_window").map(Some);};
        match self.note_window(request,WindowBinding {app_id:request.app_id.clone(),user_id:request.user_id.clone(),
            session_id:request.session_id.clone(),window_id:window_id.clone(),generation:entry.identity.generation},confirmed.focused) {
            Ok(receipt)=>Ok(Some(receipt)),Err(error)=>self.window_pending(request,&format!("p4_window_binding_refused: {error}")).map(Some),
        }
    }
    pub(crate) fn has_window_process(&self,request:&LaunchRequest)->bool {
        self.windows.lock().is_ok_and(|windows|windows.has_process(request))
    }
    fn window_pending(&self,request:&LaunchRequest,reason:&str)->io::Result<LaunchReceipt> {
        let _serial=self.serial.lock().map_err(|_|io::Error::other("launch serialization poisoned"))?;
        let mut receipt=self.receipt(request)?.ok_or_else(||invalid("unknown app window request"))?;
        if matches!(receipt.state,LaunchState::OutcomeUnknown|LaunchState::StartedWindowPending|LaunchState::Started|LaunchState::Activated) {
            // OutcomeUnknown still means the OS dispatch may have occurred. It
            // must never be promoted to Started merely because probing failed.
            if receipt.state!=LaunchState::OutcomeUnknown {receipt.state=LaunchState::StartedWindowPending;}
            receipt.window=None;receipt.reason=reason.into();self.journal.update(&receipt)?;
        }Ok(receipt)
    }
    /// Trusted local Android bridge completion, including a reply arriving after
    /// the synchronous dispatch timed out. It cannot trigger an OS operation.
    pub(crate) fn note_dispatch_result(&self,request:&LaunchRequest,dispatch_id:&str,result:super::shared::platform::DispatchReceipt)->io::Result<LaunchReceipt> {
        let _serial=self.serial.lock().map_err(|_|io::Error::other("launch serialization poisoned"))?;
        let mut receipt=self.receipt(request)?.ok_or_else(||invalid("unknown local platform completion"))?;
        if receipt.dispatch_id!=dispatch_id {return Ok(receipt);}
        if receipt.state==LaunchState::ConsentRequired && result.state==LaunchState::Denied {
            receipt.state=result.state;receipt.reason=result.reason;self.journal.update(&receipt)?;return Ok(receipt);
        }
        if receipt.state!=LaunchState::OutcomeUnknown {return Ok(receipt);}
        if result.state==LaunchState::OutcomeUnknown {return Ok(receipt);}
        if !matches!(result.state,LaunchState::StartedWindowPending|LaunchState::ConsentRequired|LaunchState::Denied|LaunchState::Unsupported) {
            return Err(invalid("platform completion cannot assert a P4 window activation"));
        }
        receipt.state=result.state;receipt.reason=result.reason;receipt.pid=result.pid;
        self.journal.update(&receipt)?;
        if receipt.state==LaunchState::ConsentRequired {self.journal.release_unstarted_continuation(&request.request_id)?;}
        Ok(receipt)
    }
    /// Called by the local P4 adapter after verified window/app/session matching.
    /// A PID/name match alone is insufficient. Timeout never calls `launch` again.
    pub(crate) fn note_window(&self, request: &LaunchRequest, window: WindowBinding,
        activated: bool) -> io::Result<LaunchReceipt> {
        let _serial=self.serial.lock().map_err(|_|io::Error::other("launch serialization poisoned"))?;
        if window.app_id!=request.app_id || window.user_id!=request.user_id
            || window.session_id!=request.session_id || window.window_id.is_empty() || window.generation==0 {
            return Err(invalid("P4 window does not belong to this launch"));
        }
        let (catalog,_) = self.snapshot()?;
        self.facts(request,&catalog).map_err(invalid)?;
        let mut receipt=self.receipt(request)?.ok_or_else(||invalid("unknown launch request"))?;
        if !matches!(receipt.state,LaunchState::StartedWindowPending|LaunchState::Started|LaunchState::Activated|LaunchState::OutcomeUnknown) {
            return Err(invalid("launch has no admitted window transition"));
        }
        receipt.state=if activated {LaunchState::Activated} else {LaunchState::Started};
        let known=receipt.reason.starts_with("known_instance_")||receipt.reason.starts_with("existing_instance_");
        receipt.reason=if activated {"existing_or_started_window_activation_confirmed"} else if known {"existing_instance_window_available_activation_not_confirmed"} else {"started_window_available"}.into();
        receipt.window=Some(window);
        self.journal.update(&receipt)?;
        Ok(receipt)
    }
}
