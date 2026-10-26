//! PREPARE and PLAN grant no image-production right. Registered external imports
//! use their separate exact credits; ACTIVATE consumes the final frame credit
//! only after the once-only Accepted contract binding.
use super::*;
use crate::screen::core::{capture_admission::SessionAdmission,worker_capture_wire::{CaptureRequest,CaptureReply,PreparedInfoWire,PlanWire}};
impl Worker {
    pub(super) fn handle_capture_request(&mut self,request:CaptureRequest)->WorkerReply {
        match self.capture_request(request) {Ok(reply)=>WorkerReply::CaptureControl(reply),Err(error)=>capture_failed(&error)}
    }
    fn capture_request(&mut self,request:CaptureRequest)->io::Result<CaptureReply> {
        if self.remote.is_closed() {
            return Err(io::Error::new(io::ErrorKind::Interrupted,"worker preparation cancelled by close"));
        }
        match request {
            CaptureRequest::Activity(active)=> {
                if let Some(prepared)=self.prepared.as_mut() {prepared.set_capture_active(active)?;}
                else if let Some(source)=self.source.as_mut() {source.set_capture_active(active)?;}
                self.capture_active=active;Ok(CaptureReply::Activity(active))
            }
            CaptureRequest::Prepare{owner,session,source,wanted,control,working,family}=> {
                if self.capture_session.is_some() {return Err(io::Error::other("worker source was already prepared"));}
                self.remote.bind_session(&session)?;
                let admission=Arc::new(SessionAdmission::from_verified_parent_unbound(&session,working,self.remote.clone(),())?);
                self.remote.bind_selection(source.clone())?;
                CONTROL.store(control,Ordering::SeqCst);self.control=control;
                let prepared=if let Some(open)=family {
                    if open.session!=session || open.control!=control
                        || crate::screen::core::family_protocol::selector(&source)!=Some(open.lifetime.as_str()) {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied,"family preparation binding changed"));
                    }
                    let (family,prepared)=super::super::host_worker_client::family_backend::WorkerFamily::prepare(open,self.backend.clone(),admission.clone(),&owner)?;
                    self.family=Some(family);prepared
                } else {self.backend.prepare_session_source(&owner,&session,&source,wanted,control,admission.clone())
                    .map_err(|refusal|io::Error::other(refusal.reason))?};
                if prepared.source_key().session()!=session {return Err(io::Error::other("prepared worker source has another session"));}
                self.remote.bind_source(prepared.source_key())?;
                let prepared = crate::screen::core::host_admission::registered(prepared, admission.clone())?;
                self.release_sink=prepared.input_sink();
                super::super::note_input_target(&source);
                let info=PreparedInfoWire::new(prepared.source_key(),prepared.description());
                self.prepared=Some(prepared);self.capture_session=Some(admission);Ok(CaptureReply::Prepared(info))
            }
            CaptureRequest::Plan(output)=> {
                let admission=self.capture_session.as_ref().ok_or_else(||io::Error::other("worker source is not prepared"))?;
                admission.bind_contract(output.contract)?;self.admitted=Some(output.contract);
                let prepared=self.prepared.as_ref().ok_or_else(||io::Error::other("worker preparation was already consumed"))?;
                let mut plan=prepared.capture_plan(&output.plan()?.for_worker())?;
                let info=PreparedInfoWire::new(prepared.source_key(),prepared.description());
                if plan.as_ref().is_some_and(|plan|plan.generation().value()!=info.generation || info.size.is_none()) {plan=None;}
                let wire=plan.as_ref().map(PlanWire::new);self.planned=plan;
                Ok(CaptureReply::Plan{info,plan:wire})
            }
            CaptureRequest::Activate{credit}=> {
                if self.admitted.is_none() {return Err(io::Error::other("worker activation contract is unbound"));}
                let plan=self.planned.take().ok_or_else(||io::Error::other("worker native metadata is pending"))?;
                let grant=self.remote.import(plan,credit)?;
                let prepared=self.prepared.take().ok_or_else(||io::Error::other("worker source is not prepared"))?;
                let mut source=prepared.activate(grant).map_err(|refusal|io::Error::other(refusal.reason))?;
                if source.capture_grant().is_none() {return Err(io::Error::other("activated worker source has no grant"));}
                self.release_sink=source.input_sink().or_else(||self.release_sink.take());
                source.set_capture_active(self.capture_active)?;
                self.reference_generation=source.reference_generation();self.source=Some(source);Ok(CaptureReply::Activated)
            }
        }
    }
}
