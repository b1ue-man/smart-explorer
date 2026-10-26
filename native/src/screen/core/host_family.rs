//! P4 messages remain bound to this session's original source/worker and proof.
use super::*;
use crate::screen::core::family_protocol::{FamilyInput,FamilyInputResult,FamilyOperation,FamilyOperationResult};
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};

pub(super) struct FamilyState {
    pub(super) supported:bool,
    pub(super) selected:Option<String>,
    revision:u64,
    layout_revision:u64,
    focused:Vec<crate::remote_window::WindowIdentity>,
}
struct Lease(Arc<AtomicBool>);
impl Drop for Lease {fn drop(&mut self) {self.0.store(false,Ordering::Release);}}
impl FamilyState {
    pub(super) fn new(supported:bool,selected:Option<String>)->Self {Self {supported,selected,revision:0,layout_revision:0,focused:Vec::new()}}
    pub(super) fn reset(&mut self,selected:Option<String>) {self.selected=selected;self.revision=0;self.layout_revision=0;self.focused.clear();}
}

async fn job<T:Send+'static>(mut stop:tokio::sync::watch::Receiver<Option<EndReason>>,
    connection:&Connection,active:Arc<AtomicBool>,work:tokio::task::JoinHandle<io::Result<T>>)->io::Result<T> {
    let _lease=Lease(active.clone());
    if stop.borrow().is_some() {active.store(false,Ordering::Release);return Err(io::Error::from(io::ErrorKind::Interrupted));}
    let result=tokio::select! {biased;
        _=stop.changed()=>Err(io::Error::from(io::ErrorKind::Interrupted)),
        _=connection.closed()=>Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
        joined=work=>joined.map_err(io::Error::other).and_then(|result|result),
    };
    active.store(false,Ordering::Release);
    result
}

impl Session<'_> {
    fn family_admitted(&self,control:bool)->io::Result<()> {
        if !self.family.supported || self.family.selected.is_none() {
            return Err(io::Error::new(io::ErrorKind::Unsupported,"source_has_no_negotiated_window_family"));
        }
        if self.handle.stop_signal().borrow().is_some()
            || !crate::screen::core::family_listing::admitted(self.context,self.principal,control) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,"family_permission_revoked"));
        }
        Ok(())
    }

    async fn publish_family(&mut self,snapshot:crate::remote_window::FamilySnapshot,force:bool)->io::Result<()> {
        snapshot.validate()?;
        if self.family.selected.as_deref()!=Some(snapshot.session.as_str()) || snapshot.revision<self.family.revision {
            return Err(io::Error::new(io::ErrorKind::InvalidData,"family_snapshot_identity_changed"));
        }
        let changed=snapshot.revision!=self.family.revision;
        if !changed && !force {return Ok(());}
        let focused=snapshot.members.iter().filter(|m|m.focused).map(|m|m.identity.clone()).collect::<Vec<_>>();
        if focused!=self.family.focused {
            self.release_pressed();
            self.family.focused=focused;
        }
        if snapshot.layout_revision!=self.family.layout_revision {
            self.pipeline.invalidate_reference();
            self.input_floor=Some(self.pipeline.next_frame_id());
            self.release_pressed();
            self.family.layout_revision=snapshot.layout_revision;
        }
        self.family.revision=snapshot.revision;
        self.frame(HostFrame::FamilySnapshot(snapshot)).await
    }

    pub(super) async fn family_snapshot(&mut self,force:bool)->io::Result<()> {
        self.family_admitted(false)?;
        let sink=self.pipeline.input_sink().ok_or_else(||io::Error::other("family_worker_unavailable"))?;
        let active=Arc::new(AtomicBool::new(true));
        let work=tokio::task::spawn_blocking(move ||sink.family_snapshot());
        let snapshot=job(self.handle.stop_signal(),self.connection,active,work).await?
            .ok_or_else(||io::Error::other("family_worker_has_no_bound_source"))?;
        self.family_admitted(false)?;
        self.publish_family(snapshot,force).await
    }

    pub(super) async fn family_operation(&mut self,request:FamilyOperation)->io::Result<()> {
        self.family_admitted(true)?;
        if !self.handle.holds_control() || self.mode!=SessionMode::Control {
            return self.frame(HostFrame::FamilyOperation(FamilyOperationResult {request_id:request.request_id,
                status:None,reason:Some("family_control_not_held".into()),snapshot:None})).await;
        }
        if self.family.selected.as_deref()!=Some(request.identity.session.as_str()) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,"foreign_family_operation"));
        }
        if request.operation==crate::remote_window::WindowOperation::Focus {
            self.release_pressed();
            if self.input_release.is_none() {
                // The worker can retain failed releases after the daemon's
                // tracker was drained. Flush that owned state before focus,
                // including retries after an OS/desktop restriction ended.
                let sink=self.pipeline.input_sink().ok_or_else(||io::Error::other("family_worker_unavailable"))?;
                self.input_release=Some(tokio::task::spawn_blocking(move||sink.release(&[])));
            }
            if let Some(release)=self.input_release.take() {
                let mut stop=self.handle.stop_signal();
                let result=tokio::select! {biased;
                    _=stop.changed()=>return Err(io::Error::from(io::ErrorKind::Interrupted)),
                    _=self.connection.closed()=>return Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
                    joined=release=>joined.unwrap_or(Err(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)),
                };
                self.release_blocked=result.err();
            }
            self.family_admitted(true)?;
            if self.release_blocked.is_some() || !self.handle.holds_control() {
                return self.frame(HostFrame::FamilyOperation(FamilyOperationResult {request_id:request.request_id,
                    status:None,reason:Some("family_input_release_pending_or_control_lost".into()),snapshot:None})).await;
            }
        }
        let sink=self.pipeline.input_sink().ok_or_else(||io::Error::other("family_worker_unavailable"))?;
        let active=Arc::new(AtomicBool::new(true));let gate=active.clone();
        let work=tokio::task::spawn_blocking(move ||sink.family_operation(request,gate));
        let result=job(self.handle.stop_signal(),self.connection,active,work).await?;
        self.family_admitted(true)?;
        if let Some(snapshot)=result.snapshot.clone() {self.publish_family(snapshot,false).await?;}
        self.frame(HostFrame::FamilyOperation(result)).await
    }

    pub(super) async fn family_input(&mut self,request:FamilyInput)->io::Result<()> {
        self.family_admitted(true)?;
        if let Some(blocked)=self.release_blocked.take() {
            return self.frame(HostFrame::FamilyInput(FamilyInputResult {blocked:Some(blocked),
                reason:Some("family_input_release_pending".into())})).await;
        }
        let reason=if !self.handle.holds_control() || self.mode!=SessionMode::Control {Some("family_control_not_held")}
            else if !self.visible || self.input_floor.is_some() || !self.pipeline.presentation_supported()
                || !self.pipeline.input_reference_ready() || self.input_release.is_some() {Some("family_waiting_for_presented_frame")}
            else if request.target.snapshot_revision!=self.family.revision
                || self.family.selected.as_deref()!=Some(request.target.identity.session.as_str()) {Some("family_input_snapshot_changed")}
            else {None};
        if let Some(reason)=reason {
            self.release_pressed();
            return self.frame(HostFrame::FamilyInput(FamilyInputResult {
                blocked:Some(crate::screen::core::wire_control::ControlBlock::ControlUnavailable),reason:Some(reason.into())})).await;
        }
        let events=request.events.clone();
        let sink=self.pipeline.input_sink().ok_or_else(||io::Error::other("family_worker_unavailable"))?;
        let active=Arc::new(AtomicBool::new(true));let gate=active.clone();
        let work=tokio::task::spawn_blocking(move ||sink.family_input(request,gate));
        let result=job(self.handle.stop_signal(),self.connection,active,work).await?;
        self.family_admitted(true)?;
        if result.blocked.is_none() {for event in &events {self.pressed.observe(event);}}
        else {self.release_pressed();}
        self.frame(HostFrame::FamilyInput(result)).await
    }
}
