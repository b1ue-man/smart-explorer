//! Trusted P5->P4 ownership transfer. This does not admit screen capture/input.
use std::{io,sync::{Arc,Weak},time::{Duration,Instant}};
use crate::apps::{LaunchRequest,LaunchReceipt,LaunchState};
use super::{owner_key,principal_grant,eio,ProvenPrincipal,ShareIrohNode};

pub(super) fn probe(node:&Arc<ShareIrohNode>,principal:&ProvenPrincipal,request:&LaunchRequest)->io::Result<Option<LaunchReceipt>> {
    node.require_sharing_active()?;
    {let state=node.auth.lock().map_err(|_|eio("authorization unavailable"))?;
        if principal_grant(&state,principal).is_none() {return Err(eio("app window relationship ended"));}}
    let host=node.apps.host(principal)?;
    if !host.bind_peer_request(request,&owner_key(principal)?,false)? {return Err(eio("app window request owner/body mismatch"));}
    let owner=crate::screen::core::family_protocol::owner(principal)?;
    host.correlate_window(request,|pin| {
        node.require_sharing_active()?;
        {let state=node.auth.lock().map_err(|_|eio("authorization unavailable"))?;
            if principal_grant(&state,principal).is_none() {return Err(eio("app window relationship ended before pin transfer"));}}
        node.screen.register_app_pin(&owner,request.request_id.as_str(),pin)
    })
}
pub(super) fn watch(node:&Arc<ShareIrohNode>,principal:ProvenPrincipal,request:LaunchRequest) {
    let id=request.request_id.to_string();
    if !node.apps.window_watches.lock().is_ok_and(|mut active|active.insert(id.clone())) {return;}
    let weak=Arc::downgrade(node);let cleanup=weak.clone();let cleanup_id=id.clone();
    if std::thread::Builder::new().name("app-window-correlation".into()).spawn(move|| {
        follow(&weak,&principal,&request);
        if let Some(node)=weak.upgrade() {if let Ok(mut active)=node.apps.window_watches.lock() {active.remove(&id);}}
    }).is_err() {if let Some(node)=cleanup.upgrade() {if let Ok(mut active)=node.apps.window_watches.lock() {active.remove(&cleanup_id);}}}
}
fn follow(node:&Weak<ShareIrohNode>,principal:&ProvenPrincipal,request:&LaunchRequest) {
    // Same 30s interactive command horizon and 500ms status cadence as the
    // existing daemon notice path. Afterwards every Receipt performs one fresh
    // observation; expiry never becomes another Launch operation.
    let end=Instant::now()+Duration::from_secs(30);
    while Instant::now()<end {
        let Some(node)=node.upgrade() else {return;};
        match probe(&node,principal,request) {
            Ok(Some(receipt)) if receipt.window.is_some()||!matches!(receipt.state,LaunchState::StartedWindowPending|LaunchState::OutcomeUnknown)=>return,
            Err(_)=>return,
            _=>{},
        }
        if !node.apps.host(principal).is_ok_and(|host|host.has_window_process(request)) {return;}
        drop(node);std::thread::sleep(Duration::from_millis(500));
    }
}
