//! Catalog-bound app authority and durable peer ownership; no Exec fallback.
use std::{collections::BTreeMap,io,path::PathBuf,sync::{Arc,Mutex,Weak,atomic::{AtomicBool,Ordering}},time::Duration};
use iroh::endpoint::Connection;
use tokio::sync::Semaphore;
use sha2::{Digest,Sha256};
use crate::apps::{AppHost,CatalogIcons,DeviceId,LaunchRequest,LaunchReceipt,LaunchAdmission,LaunchDenial,LaunchFacts,CatalogSnapshot};
use super::{apps_wire::{self,AppsCommand,AppsResult,AppsReply,AppsServerHello,AppsClientHello},
    access_constraints::{self as constraints,AccessRequest},access_authorize::{request_for,principal_grant,capability_for},
    access_grant::{Capability,CapabilityDecision},access_approval::{ApprovalSummary,ApprovalOutcome},
    access_revocation::{AccessConfirmation,AccessHandle},core::{eio,now_secs,random_token},
    node::ShareIrohNode,relation_proof::{prove_relation,ProvenPrincipal},session::PeerPrincipal,
    handshake_limits::ApplicationHandshakePermit};
#[path="node_apps_window.rs"] mod window;

struct Configuration {host:Arc<AppHost>}
#[derive(Clone)]
struct PendingApp {principal:ProvenPrincipal,request:LaunchRequest,access:Option<Arc<AccessHandle>>}
pub(crate) struct NodeApps {
    configuration:Mutex<Option<Configuration>>,
    pending:Mutex<BTreeMap<String,PendingApp>>,
    window_watches:Mutex<std::collections::BTreeSet<String>>,
    pub(super) slots:Arc<Semaphore>,
}
impl NodeApps {
    pub(super) fn new()->Self {Self {configuration:Mutex::new(None),pending:Mutex::new(BTreeMap::new()),window_watches:Mutex::new(Default::default()),slots:Arc::new(Semaphore::new(16))}}
    fn host(&self,principal:&ProvenPrincipal)->io::Result<Arc<AppHost>> {
        let _=principal;
        self.local_host()
    }
    fn local_host(&self)->io::Result<Arc<AppHost>> {
        let configuration=self.configuration.lock().map_err(|_|eio("app host configuration unavailable"))?;
        let config=configuration.as_ref().ok_or_else(||io::Error::new(io::ErrorKind::Unsupported,"interactive app host is not installed"))?;
        Ok(config.host.clone())
    }
}
fn owner_key(principal:&ProvenPrincipal)->io::Result<String> {
    let bytes=serde_json::to_vec(&(&principal.relation_kind,&principal.relation_id,&principal.device_id,
        &principal.public_key,&principal.fingerprint,&principal.node_id)).map_err(io::Error::other)?;
    Ok(format!("{:x}",Sha256::digest(bytes)))
}
pub(super) fn stop_all(apps:&NodeApps) {if let Ok(mut pending)=apps.pending.lock() {pending.clear();}}
impl super::service::ShareService {
    /// Local daemon-token callers only. No network command exposes this inventory.
    pub(crate) fn local_app_grants(&self,device:&str,key:&str)->io::Result<crate::apps::LocalAppGrantSnapshot> {
        let catalog=self.iroh.apps.local_host()?.catalog()?;
        let own=crate::share::exec_provider_status().user_label;
        let rights=self.device_access_view(device,key,&own,now_secs()).map_err(eio)?;
        Ok(crate::apps::LocalAppGrantSnapshot {peer_device_id:device.into(),peer_public_key:key.into(),catalog,relations:rights.relations})
    }
    /// Build a minimal AppLaunch patch from current host facts/current exact
    /// source. Persistence repeats the supplied revision; every unrelated field
    /// survives because account/programs/global expiry are omitted and the
    /// other V2 conditions are copied at that same revision.
    pub(crate) fn prepare_local_app_grant(&self,change:&crate::apps::LocalAppGrantChange)->io::Result<super::access_grant::AccessGrantChange> {
        use super::{access_grant::AccessGrantChange,access_constraints::AppLaunchConstraints};
        let (device,key)=match &change.target {super::types::ExecGrantTarget::Direct {device_id,public_key,..}
            |super::types::ExecGrantTarget::RoomMember {device_id,public_key,..}=>(device_id.as_str(),public_key.as_str())};
        if change.host_device_id.as_str()!=self.identity.device_id {return Err(eio("local_host_identity_changed; reload app grant editor"));}
        let own=crate::share::exec_provider_status().user_label;
        let rights=self.device_access_view(device,key,&own,now_secs()).map_err(eio)?;
        let source=rights.relations.into_iter().find(|r|r.target==change.target).ok_or_else(||eio("exact app grant relationship no longer exists"))?;
        if source.policy_revision!=change.expected_policy_revision {return Err(eio("app grant policy revision changed; reload without overwriting"));}
        let mut patch=AccessGrantChange {capabilities:BTreeMap::from([(Capability::AppLaunch,change.decision)]),..Default::default()};
        // Revoking never needs a working GUI/catalog and leaves the previous
        // bounds intact. Re-enabling always needs a fresh explicit binding.
        if change.decision==CapabilityDecision::Off {return Ok(patch);}
        if !source.active {return Err(eio("app grant relationship is inactive"));}
        let catalog=self.iroh.apps.local_host()?.catalog()?;
        if catalog.device_id!=change.host_device_id||catalog.revision!=change.catalog_revision
            ||catalog.session.user_id!=change.user_id||catalog.authorization_scope!=change.authorization_scope {
            return Err(eio("local_app_catalog_or_login_changed; explicit rebinding required"));
        }
        if !catalog.session.interactive {return Err(eio("local host has no interactive app session"));}
        if change.app_ids.is_empty()||change.app_ids.len()>catalog.apps.len()
            ||change.app_ids.iter().collect::<std::collections::BTreeSet<_>>().len()!=change.app_ids.len()
            ||change.app_ids.iter().any(|id|!catalog.apps.iter().any(|app|app.app_id==*id&&app.can_launch)) {
            return Err(eio("select explicit available IDs from the current local app catalog"));
        }
        if change.valid_until.is_some_and(|end|end<=now_secs()) {return Err(eio("app grant expiry must be in the future"));}
        let mut constraints=source.grant.constraints;
        constraints.app_launch=Some(AppLaunchConstraints {
            app_ids:change.app_ids.iter().map(|id|constraints::AppId::parse(id.as_str()).map_err(eio)).collect::<io::Result<_>>()?,
            sessions:vec![constraints::SessionId::parse(catalog.authorization_scope.as_str()).map_err(eio)?],
            users:vec![constraints::UserId::parse(catalog.session.user_id.as_str()).map_err(eio)?],valid_until:change.valid_until,
        });
        constraints.validate().map_err(eio)?;patch.constraints=Some(constraints);Ok(patch)
    }
    pub(crate) fn configure_apps_host(&self,device:DeviceId,root:PathBuf,icons:Option<CatalogIcons>)->io::Result<()> {
        if device.as_str()!=self.identity.device_id {return Err(eio("app host device identity mismatch"));}
        let mut configuration=self.iroh.apps.configuration.lock().map_err(|_|eio("app host configuration unavailable"))?;
        if configuration.is_some() {return Ok(());}
        let host=AppHost::for_current_user(device,&root,icons)?;
        install_platform(Arc::downgrade(&self.iroh))?;
        *configuration=Some(Configuration {host:Arc::new(host)});
        Ok(())
    }
}
impl ShareIrohNode {
    pub(super) async fn handle_apps_connection(self:Arc<Self>,connection:Connection,permit:ApplicationHandshakePermit)->io::Result<()> {
        let _close=super::node_tcp::CloseTcp(connection.clone());
        let _incoming=self.track_incoming(&connection)?;
        let _slot=self.apps.slots.clone().try_acquire_owned().map_err(|_|eio("device-command admission exhausted"))?;
        self.require_sharing_active()?;super::node_tcp::require_policy(&connection)?;
        self.await_route(&connection,None,None).await?;
        super::route_protocol::scope(connection.clone(),self.serve_apps(connection,permit)).await
    }
    async fn serve_apps(self:&Arc<Self>,connection:Connection,permit:ApplicationHandshakePermit)->io::Result<()> {
        let deadline=super::route_protocol::active_now()+Duration::from_secs(20);
        let (mut send,mut recv)=super::route_protocol::run_until_active(deadline,"apps hello timeout",async {connection.open_bi().await.map_err(eio)}).await?;
        let identity=self.auth.lock().map_err(|_|eio("authorization unavailable"))?.identity.clone();
        let server=AppsServerHello {version:apps_wire::APPS_VERSION,challenge:format!("rp1.{}",random_token(32).map_err(eio)?),
            device_id:identity.device_id,public_key:identity.public_key,fingerprint:identity.fingerprint,node_id:identity.node_id};
        super::route_protocol::run_until_active(deadline,"apps server hello timeout",apps_wire::send(&mut send,&server)).await?;
        let remote=connection.remote_id().to_string();
        let client:AppsClientHello=super::route_protocol::run_until_active(deadline,"apps client hello timeout",apps_wire::receive_hello(&mut recv)).await
            .map_err(|error| {self.auth_penalties().record(&remote,super::auth_penalty::PenaltyKind::NoAuthentication,std::time::Instant::now());error})?;
        client.validate()?;
        let principal={let state=self.auth.lock().map_err(|_|eio("authorization unavailable"))?;
            prove_relation(&server.relation(&client)?,&client.relation(),&remote,&state)
        }.map_err(|rejection| {
            if rejection.counts_as_failed_authentication() {self.auth_penalties().record(&remote,super::auth_penalty::PenaltyKind::AuthFailure,std::time::Instant::now());}
            io::Error::new(io::ErrorKind::PermissionDenied,"device relation proof refused")
        })?;
        self.bind_incoming_principal(&connection,PeerPrincipal::new(&principal.relation_kind,&principal.relation_id,
            &principal.device_id,&principal.public_key,&principal.node_id)).await?;
        super::route_monitor::authenticated_remote_capability(&connection,client.nonce.starts_with("rp1."));drop(permit);
        let result=self.apps_execute(&connection,&principal,&client.command,&mut send).await;
        let mut reply=match result {Ok(result)=>AppsReply::Completed {id:client.id.clone(),request_hash:client.request_hash()?,result},
            Err(error)=>AppsReply::Refused {detail:error.to_string()}};
        if serde_json::to_vec(&reply).map_err(eio)?.len()>apps_wire::MAX_PACKET {
            reply=AppsReply::Refused {detail:"complete_device_inventory_exceeds_ipc_frame_budget; no partial catalog or RDP inventory returned".into()};
        }
        super::route_protocol::timeout_for(Duration::from_secs(20),apps_wire::send(&mut send,&reply)).await??;
        send.finish().map_err(eio)?;
        let _=tokio::time::timeout(Duration::from_secs(2),connection.closed()).await;Ok(())
    }
    async fn apps_execute(self:&Arc<Self>,connection:&Connection,principal:&ProvenPrincipal,command:&AppsCommand,
        send:&mut iroh::endpoint::SendStream)->io::Result<AppsResult> {
        self.require_sharing_active()?;
        match command {
            AppsCommand::RdpProbe=>{
                self.require_inventory(principal)?;
                let host=tokio::task::spawn_blocking(crate::rdp::probe_local_host).await.map_err(eio)?;
                self.require_inventory(principal)?;Ok(AppsResult::RdpProbe {host})
            },
            AppsCommand::Catalog=>{
                let host=self.apps.host(principal)?;let catalog=tokio::task::spawn_blocking(move||host.catalog()).await.map_err(eio)??;
                Ok(AppsResult::Catalog {catalog:self.visible_catalog(principal,catalog)?})
            },
            AppsCommand::Icon {app_id,icon,catalog_revision}=>{
                icon.validate()?;
                let host=self.apps.host(principal)?;
                let catalog={let host=host.clone();tokio::task::spawn_blocking(move||host.catalog()).await.map_err(eio)??};
                let visible=self.visible_catalog(principal,catalog)?;
                if &visible.revision!=catalog_revision||!visible.apps.iter().any(|a|&a.app_id==app_id&&a.icon.as_ref()==Some(icon)) {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied,"icon does not belong to the current authorized app catalog"));
                }
                let id=app_id.clone();let expected=icon.clone();
                let renderer=host.clone();
                let reply=tokio::task::spawn_blocking(move||renderer.icon(&id,&expected)).await.map_err(eio)??;
                let fresh=tokio::task::spawn_blocking(move||host.catalog()).await.map_err(eio)??;
                let fresh=self.visible_catalog(principal,fresh)?;
                if &fresh.revision!=catalog_revision||!fresh.apps.iter().any(|app|&app.app_id==app_id&&app.icon.as_ref()==Some(icon)) {
                    return Err(eio("app icon catalog changed while rendering"));
                }
                Ok(AppsResult::Icon {reply})
            },
            AppsCommand::Receipt {request}=>{
                let host=self.apps.host(principal)?;
                if !host.bind_peer_request(request,&owner_key(principal)?,false)? {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"app receipt owner/request does not match"));}
                let node=self.clone();let proven=principal.clone();let request=request.clone();
                let receipt=tokio::task::spawn_blocking(move||window::probe(&node,&proven,&request)).await.map_err(eio)??;
                // The owner-specific persistent ledger is unreachable from a
                // different proven relationship, even after a worker restart.
                if principal_grant(&self.auth.lock().map_err(|_|eio("authorization unavailable"))?,principal).is_none() {return Err(eio("relationship ended"));}
                Ok(AppsResult::Receipt {receipt})
            },
            AppsCommand::Launch {request}=>{
                let host=self.apps.host(principal)?;
                if !host.bind_peer_request(request,&owner_key(principal)?,true)? {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"app request ID owner or contents differ"));}
                let catalog={let host=host.clone();tokio::task::spawn_blocking(move||host.catalog()).await.map_err(eio)??};
                let access=access_request(request,&catalog)?;
                let confirmation=self.apps_confirmation(Some(connection),principal,&access,Some(send)).await?;
                let node=self.clone();let principal=principal.clone();let request=request.clone();
                let live=Arc::new(AtomicBool::new(true));let dispatch_live=live.clone();
                let mut job=tokio::task::spawn_blocking(move||node.run_app(&principal,&request,false,confirmation,Some(dispatch_live)));
                let receipt=tokio::select! {
                    result=&mut job=>result.map_err(eio)??,
                    _=connection.closed()=>{live.store(false,Ordering::Release);let _=job.await;return Err(eio("app requester disconnected before completion"));}
                };
                Ok(AppsResult::Launch {receipt})
            },
        }
    }
    fn require_inventory(&self,principal:&ProvenPrincipal)->io::Result<()> {
        let state=self.auth.lock().map_err(|_|eio("authorization unavailable"))?;
        if capability_for(&state,principal,Capability::ScreenView,now_secs()).decision!=CapabilityDecision::Allow {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,"host inventory requires explicit screen-view metadata permission"));
        }Ok(())
    }
    fn visible_catalog(&self,principal:&ProvenPrincipal,mut catalog:CatalogSnapshot)->io::Result<CatalogSnapshot> {
        let state=self.auth.lock().map_err(|_|eio("authorization unavailable"))?;
        let grant=principal_grant(&state,principal).ok_or_else(||eio("relationship ended"))?.access_grant();
        if grant.expired_at(now_secs())||!grant.decision(Capability::AppLaunch).is_on() {return Err(eio("app catalog disclosure is not authorized"));}
        catalog.apps.retain(|app| {
            let (Ok(app_id),Ok(session),Ok(user))=(constraints::AppId::parse(app.app_id.as_str()),
                constraints::SessionId::parse(catalog.authorization_scope.as_str()),constraints::UserId::parse(catalog.session.user_id.as_str())) else {return false;};
            grant.decision_for(&AccessRequest::AppLaunch(constraints::AppLaunchRequest {app_id,session,user}),now_secs()).is_on()
        });
        if catalog.apps.is_empty() {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"no app catalog entries are granted for this login/catalog scope"));}
        Ok(catalog)
    }
    fn run_app(self:&Arc<Self>,principal:&ProvenPrincipal,request:&LaunchRequest,resume:bool,mut confirmation:Option<AccessConfirmation>,live:Option<Arc<AtomicBool>>)->io::Result<LaunchReceipt> {
        let host=self.apps.host(principal)?;
        let owner=owner_key(principal)?;
        if !host.bind_peer_request(request,&owner,true)? {return Err(eio("app request owner or contents differ"));}
        if crate::apps::os::host::local_consent_supported() {
            let prior=self.apps.pending.lock().map_err(|_|eio("app context unavailable"))?.get(request.request_id.as_str()).cloned();
            if prior.as_ref().is_some_and(|prior|prior.request!=*request||!owner_key(&prior.principal).is_ok_and(|key|key==owner)) {return Err(eio("app consent context ownership mismatch"));}
            if let Some(receipt)=host.receipt(request)? {
                if receipt.state==crate::apps::LaunchState::ConsentRequired && (prior.is_none()||prior.as_ref().and_then(|p|p.access.as_ref()).is_some_and(|a|a.reason().is_some())) {
                    let denied=host.note_dispatch_result(request,&receipt.dispatch_id,crate::apps::os::shared::platform::DispatchReceipt {
                        state:crate::apps::LaunchState::Denied,reason:"original_app_admission_ended_or_worker_restarted".into(),pid:None})?;
                    crate::apps::os::host::end_platform_consent(request,&denied.reason);return Ok(denied);
                }
            }
            self.apps.pending.lock().map_err(|_|eio("app context unavailable"))?.insert(request.request_id.to_string(),
                PendingApp {principal:principal.clone(),request:request.clone(),access:prior.and_then(|p|p.access)});
        }
        let authorize=|facts:&LaunchFacts| {
            let access=access_request(&facts.request,&facts.catalog).map_err(|e|LaunchDenial::Denied(e.to_string()))?;
            let state=self.auth.lock().map_err(|_|LaunchDenial::Denied("authorization unavailable".into()))?;
            self.require_sharing_active().map_err(|e|LaunchDenial::Denied(e.to_string()))?;
            if live.as_ref().is_some_and(|live|!live.load(Ordering::Acquire)) {return Err(LaunchDenial::Denied("app requester cancelled".into()));}
            if self.apps.pending.lock().map_err(|_|LaunchDenial::Denied("app context unavailable".into()))?.get(request.request_id.as_str())
                .and_then(|pending|pending.access.as_ref()).is_some_and(|access|access.reason().is_some()) {
                return Err(LaunchDenial::Denied("original app admission was revoked".into()));
            }
            let handle={
                if resume {
                    if request_for(&state,principal,&access,now_secs()).decision==CapabilityDecision::Off {return Err(LaunchDenial::Denied("app grant ended".into()));}
                    self.apps.pending.lock().map_err(|_|LaunchDenial::Denied("app context unavailable".into()))?.get(request.request_id.as_str())
                        .and_then(|pending|pending.access.clone()).filter(|handle|handle.reason().is_none())
                        .ok_or_else(||LaunchDenial::Denied("original app confirmation is no longer active".into()))?
                } else {
                    Arc::new(self.access_registry().admit(&state,principal,access.clone(),confirmation.take(),now_secs())
                        .map_err(|reason|LaunchDenial::Denied(format!("app admission refused: {reason:?}")))?)
                }
            };
            let grant=principal_grant(&state,principal).ok_or_else(||LaunchDenial::Denied("relationship ended".into()))?.access_grant();
            let end=match (grant.valid_until,grant.constraints.end_for(&access,now_secs())) {(Some(a),Some(b))=>Some(a.min(b)),(a,b)=>a.or(b)};
            let authority=handle.clone();let node=Arc::downgrade(self);let live=live.clone();
            if let Some(pending)=self.apps.pending.lock().map_err(|_|LaunchDenial::Denied("app context unavailable".into()))?.get_mut(request.request_id.as_str()) {pending.access=Some(handle.clone());}
            let valid=Arc::new(move||authority.reason().is_none()&&live.as_ref().is_none_or(|live|live.load(Ordering::Acquire))
                &&node.upgrade().is_some_and(|n|n.require_sharing_active().is_ok()));
            Ok(LaunchAdmission {guard:(state,handle),expires_at_unix_ms:end.map(|end|end.saturating_mul(1000)),valid:Some(valid)})
        };
        let mut receipt=if resume {host.resume_after_local_consent(request,authorize)?} else {host.launch(request,authorize)?};
        if host.has_window_process(request)&&matches!(receipt.state,crate::apps::LaunchState::StartedWindowPending|crate::apps::LaunchState::OutcomeUnknown) {
            if let Ok(Some(confirmed))=window::probe(self,principal,request) {receipt=confirmed;}
            if receipt.window.is_none() {window::watch(self,principal.clone(),request.clone());}
        }
        if !matches!(receipt.state,crate::apps::LaunchState::ConsentRequired|crate::apps::LaunchState::OutcomeUnknown) {
            self.apps.pending.lock().map_err(|_|eio("app consent ownership unavailable"))?.remove(request.request_id.as_str());
        }
        Ok(receipt)
    }
    async fn apps_confirmation(&self,connection:Option<&Connection>,principal:&ProvenPrincipal,request:&AccessRequest,
        send:Option<&mut iroh::endpoint::SendStream>)->io::Result<Option<AccessConfirmation>> {
        let capability={let state=self.auth.lock().map_err(|_|eio("authorization unavailable"))?;request_for(&state,principal,request,now_secs())};
        match capability.decision {CapabilityDecision::Allow=>return Ok(None),CapabilityDecision::Off=>return Err(eio("app launch is off")),CapabilityDecision::Ask=>{}}
        let mut ticket=super::consent_launch::approvals().request_access(principal,request.clone(),ApprovalSummary {policy_revision:capability.policy_revision,..Default::default()});
        if let Some(send)=send {super::route_protocol::timeout_for(Duration::from_secs(20),apps_wire::send(send,&AppsReply::Pending)).await??;}
        loop {tokio::select! {
            _=async {if let Some(connection)=connection {connection.closed().await;} else {std::future::pending::<()>().await;}}=>return Err(eio("app requester disconnected")),
            outcome=ticket.outcome()=>return match outcome {ApprovalOutcome::Once|ApprovalOutcome::Always=>Ok(ticket.take_confirmation()),_=>Err(eio("app confirmation refused"))},
            _=tokio::time::sleep(Duration::from_secs(1))=>{
                self.require_sharing_active()?;
                let state=self.auth.lock().map_err(|_|eio("authorization unavailable"))?;
                let current=request_for(&state,principal,request,now_secs());
                if current.decision==CapabilityDecision::Off||current.policy_revision!=capability.policy_revision {return Err(eio("app policy changed during confirmation"));}
            }
        }}
    }
}
fn access_request(request:&LaunchRequest,catalog:&CatalogSnapshot)->io::Result<AccessRequest> {
    if request.target_device_id!=catalog.device_id||request.user_id!=catalog.session.user_id
        ||request.session_id!=catalog.session.session_id||request.catalog_revision!=catalog.revision
        ||!catalog.apps.iter().any(|a|a.app_id==request.app_id) {return Err(eio("app request does not match fresh host catalog/session"));}
    Ok(AccessRequest::AppLaunch(constraints::AppLaunchRequest {app_id:constraints::AppId::parse(request.app_id.as_str()).map_err(eio)?,
        session:constraints::SessionId::parse(catalog.authorization_scope.as_str()).map_err(eio)?,user:constraints::UserId::parse(catalog.session.user_id.as_str()).map_err(eio)?}))
}
fn platform_node()->&'static Mutex<Weak<ShareIrohNode>> {static NODE:std::sync::OnceLock<Mutex<Weak<ShareIrohNode>>>=std::sync::OnceLock::new();NODE.get_or_init(||Mutex::new(Weak::new()))}
fn install_platform(node:Weak<ShareIrohNode>)->io::Result<()> {
    *platform_node().lock().map_err(|_|eio("app callback owner unavailable"))?=node;
    crate::apps::os::host::install_platform_hooks(platform_result,platform_consent)
}
fn platform_result(request:LaunchRequest,dispatch:String,result:crate::apps::os::shared::platform::DispatchReceipt) {
    let Some(node)=platform_node().lock().ok().and_then(|current|current.upgrade()) else {return;};
    let context=node.apps.pending.lock().ok().and_then(|pending|pending.get(request.request_id.as_str()).cloned());
    let Some(context)=context.filter(|context|context.request==request) else {return;};
    let principal=context.principal;let original=context.request;
    // The JNI call must return immediately; the host owns its serialization
    // guard while Kotlin is delivering this result.
    let _=std::thread::Builder::new().name("app-local-result".into()).spawn(move|| {
        if let Ok(host)=node.apps.host(&principal) {
            if host.bind_peer_owner(&original.request_id,&owner_key(&principal).unwrap_or_default(),false).unwrap_or(false) {
                if let Ok(receipt)=host.note_dispatch_result(&original,&dispatch,result) {
                    if receipt.dispatch_id==dispatch&&!matches!(receipt.state,crate::apps::LaunchState::ConsentRequired|crate::apps::LaunchState::OutcomeUnknown) {
                        if let Ok(mut pending)=node.apps.pending.lock() {pending.remove(original.request_id.as_str());}
                    }
                }
            }
        }
    });
}
fn platform_consent(request:LaunchRequest) {
    let Some(node)=platform_node().lock().ok().and_then(|current|current.upgrade()) else {return;};
    let context=node.apps.pending.lock().ok().and_then(|pending|pending.get(request.request_id.as_str()).cloned());
    let Some(context)=context.filter(|context|context.request==request) else {
        crate::apps::os::host::end_platform_consent(&request,"original_app_peer_context_unavailable");return;
    };
    let principal=context.principal;let original=context.request;let original_access=context.access;
    let runtime=node.rt.clone();
    runtime.spawn(async move {
        let result=async {
            if original_access.as_ref().is_none_or(|access|access.reason().is_some()) {return Err(eio("original app admission was revoked or expired"));}
            let host=node.apps.host(&principal)?;
            let catalog=tokio::task::spawn_blocking(move||host.catalog()).await.map_err(eio)??;
            let access=access_request(&original,&catalog)?;
            // The original contextual Ask approval remains represented by its
            // retained AccessHandle. Recheck it and the exact current policy;
            // a local Android BAL confirmation never creates a new E4 grant.
            {
                let state=node.auth.lock().map_err(|_|eio("authorization unavailable"))?;
                if request_for(&state,&principal,&access,now_secs()).decision==CapabilityDecision::Off {return Err(eio("app grant ended while awaiting local consent"));}
            }
            let node=node.clone();let principal=principal.clone();let original=original.clone();
            tokio::task::spawn_blocking(move||node.run_app(&principal,&original,true,None,None)).await.map_err(eio)?
        }.await;
        if let Err(error)=result {
            if let Ok(host)=node.apps.host(&principal) {
                if let Ok(Some(receipt))=host.receipt(&original) {
                    let _=host.note_dispatch_result(&original,&receipt.dispatch_id,crate::apps::os::shared::platform::DispatchReceipt {
                        state:crate::apps::LaunchState::Denied,reason:format!("local_app_consent_refused: {error}"),pid:None});
                }
            }
            crate::apps::os::host::end_platform_consent(&original,&error.to_string());
        } else if let Ok(receipt)=result {
            if matches!(receipt.state,crate::apps::LaunchState::Denied|crate::apps::LaunchState::Unsupported) {
                crate::apps::os::host::end_platform_consent(&original,&receipt.reason);
            }
        }
    });
}
