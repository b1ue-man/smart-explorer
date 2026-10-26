//! Resource request/cancel sequencing independent of ordinary worker asks.
use super::*;
impl RemoteAuthority {
    fn request(&self,source:&SourceKey,registration:bool,build:impl FnOnce(u64)->GrantRequest)->io::Result<GrantValue> {
        let (sender,answer)=mpsc::channel();
        let request={
            let mut state=self.state.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?;
            if state.ended || (!registration && !state.allowed.contains(source.source())) {
                return Err(io::Error::new(io::ErrorKind::Interrupted,"capture source control is closed"));
            }
            let id=state.next;state.next=id.checked_add(1).ok_or_else(||io::Error::other("worker credit request IDs exhausted"))?;
            state.pending.try_reserve(1).map_err(|_|io::Error::other("worker request registry allocation failed"))?;
            state.pending.insert(id,Pending{source:source.source().into(),registration,answer:sender});id
        };
        let request_payload = match crate::screen::core::wire::to_json(&build(request)) {
            Ok(payload) => payload,
            Err(error) => {
                self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pending.remove(&request);
                return Err(error);
            }
        };
        let sent = self.output.lock().map_err(|_|io::Error::other("worker output poisoned"))
            .and_then(|mut output| {
                let pending = self.state.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?
                    .pending.contains_key(&request);
                if !pending {
                    return Err(io::Error::new(io::ErrorKind::Interrupted,"capture request cancelled before send"));
                }
                worker_wire::write_message(&mut *output,wire::GRANT_REQUEST,&request_payload)
            });
        if let Err(error)=sent {self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pending.remove(&request);return Err(error);}
        let result=answer.recv_timeout(Duration::from_secs(30));
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pending.remove(&request);
        result.map_err(|error| io::Error::new(match error {
            mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
            mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::BrokenPipe,
        }, format!("Parent capture credit: {error}")))?
    }
    fn prepare_owner_request(&self, source: &SourceKey, owned: bool) -> io::Result<GrantValue> {
        let root = self.root.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?.clone();
        let parent = root.as_ref().filter(|root| root.as_str() != source.source()).cloned();
        let selection = if parent.is_none() {
            self.selection.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?.clone()
        } else {None};
        let value = self.request(source,true,|request| {
            if owned {GrantRequest::OwnPrepared {request,source:source.source().into(),parent,selection}}
            else {GrantRequest::RegisterPrepared {request,source:source.source().into(),parent,selection}}
        })?;
        if root.is_none() {self.bind_source(source)?;}
        let live = self.state.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?
            .allowed.contains(source.source());
        if !live {self.release_value(value);return Err(io::Error::new(io::ErrorKind::Interrupted,"prepared owner cancelled"));}
        Ok(value)
    }
    fn send_prepared_end(&self, request: GrantRequest) {
        if let Ok(payload) = crate::screen::core::wire::to_json(&request) {
            if let Ok(mut output) = self.output.lock() {
                let _ = worker_wire::write_message(&mut *output,wire::GRANT_REQUEST,&payload);
            }
        }
    }
    fn end_connection_wire(&self,source:&SourceKey,generation:u64) {
        if let Ok(payload)=crate::screen::core::wire::to_json(&GrantRequest::EndImports{source:source.source().into(),connection:generation}) {
            if let Ok(mut output)=self.output.lock() {let _=worker_wire::write_message(&mut *output,wire::GRANT_REQUEST,&payload);}
        }
    }
}
impl CaptureAuthority for RemoteAuthority {
    fn approve(&self,plan:CapturePlan)->io::Result<CaptureGrant> {
        let value=self.request(plan.source(),false,|request|GrantRequest::FramePlan{request,plan:PlanWire::new(&plan)})?;
        match value {GrantValue::Frame(credit)=>self.import(plan,credit),value=>{self.release_value(value);Err(io::Error::other("Parent returned another credit class"))}}
    }
    fn approve_supplement(&self,image:&CaptureGrant,plan:CapturePlan)->io::Result<CaptureGrant> {
        image.check_supplement(&plan)?;
        let image=self.id_for(image)?;
        let value=self.request(plan.source(),false,|request|GrantRequest::SupplementPlan {
            request,image,plan:PlanWire::new(&plan)})?;
        match value {
            GrantValue::Supplement(credit)=>self.import(plan,credit),
            value=>{self.release_value(value);Err(io::Error::other("Parent returned another supplement credit class"))}
        }
    }
    fn register_prepared_source(&self, source: SourceKey) -> io::Result<()> {
        let value = self.prepare_owner_request(&source,false)?;
        if !matches!(value,GrantValue::Registered) {
            self.release_value(value);
            return Err(io::Error::other("Parent did not register prepared source"));
        }
        Ok(())
    }
    fn unregister_prepared_source(&self, source: &SourceKey) {
        let last = {
            let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.manual.remove(source.source());
            !state.owners.contains_key(source.source())
        };
        if last {self.cancel_pending(source);}
        self.send_prepared_end(GrantRequest::EndPrepared {source:source.source().into()});
    }
    fn own_prepared_source(&self, source: SourceKey) -> io::Result<PreparedSourceOwnerId> {
        let value = self.prepare_owner_request(&source,true)?;
        match value {
            GrantValue::PreparedOwner {source:bound,owner} if bound == source.source() => PreparedSourceOwnerId::new(owner),
            value => {self.release_value(value);Err(io::Error::other("Parent returned another prepared owner"))}
        }
    }
    fn release_prepared_source(&self, source: &SourceKey, owner: PreparedSourceOwnerId) {
        let last = {
            let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(owners) = state.owners.get_mut(source.source()) {owners.retain(|id| *id != owner);}
            state.owners.retain(|_,owners| !owners.is_empty());
            !state.owners.contains_key(source.source()) && !state.manual.contains(source.source())
        };
        if last {self.cancel_pending(source);}
        self.send_prepared_end(GrantRequest::EndPreparedOwner {source:source.source().into(),owner:owner.value()});
    }
    fn begin_import_connection(&self,source:&SourceKey)->io::Result<u64> {
        let value=self.request(source,false,|request|GrantRequest::BeginImports{request,source:source.source().into()})?;
        let GrantValue::Connection(generation)=value else {self.release_value(value);return Err(io::Error::other("Parent did not open import connection"));};
        let mut state=self.state.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?;
        if !state.allowed.contains(source.source()) || state.ended {
            drop(state);self.end_connection_wire(source,generation);return Err(io::Error::new(io::ErrorKind::Interrupted,"import connection closed during opening"));
        }
        state.connections.insert(source.source().into(),generation);Ok(generation)
    }
    fn end_import_connection(&self,source:&SourceKey,generation:u64) {
        let mut state=self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.connections.get(source.source())!=Some(&generation) {return;}
        state.connections.remove(source.source());drop(state);
        self.cancel_pending(source);self.end_connection_wire(source,generation);
    }
    fn cancel_pending(&self,source:&SourceKey) {
        let mut state=self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.allowed.remove(source.source());
        while let Some(request)=state.pending.iter().find(|(_,pending)|pending.source==source.source()).map(|(id,_)|*id) {
            if let Some(pending)=state.pending.remove(&request) {let _=pending.answer.send(Err(io::Error::new(io::ErrorKind::Interrupted,"capture source request cancelled")));}
        }
    }
    fn approve_import(&self,need:CaptureImportNeed)->io::Result<CaptureImportGrant> {
        {
            let state=self.state.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?;
            if state.connections.get(need.source().source())!=Some(&need.id().connection()) {return Err(io::Error::other("prepared import connection is stale"));}
        }
        let value=self.request(need.source(),false,|request|GrantRequest::PreparedImport{request,need:ImportNeedWire::new(&need)})?;
        let GrantValue::Import(credit)=value else {self.release_value(value);return Err(io::Error::other("Parent returned a frame credit for an import"));};
        let state=self.state.lock().map_err(|_|io::Error::other("worker dispatcher poisoned"))?;
        if !state.allowed.contains(need.source().source()) || state.connections.get(need.source().source())!=Some(&need.id().connection()) {
            drop(state);self.release_unused(credit.id);return Err(io::Error::new(io::ErrorKind::Interrupted,"import approved after close"));
        }
        drop(state);let grant=self.import(need.control_plan()?,credit)?;
        CaptureImportGrant::from_control_grant(need,grant)
    }
}
