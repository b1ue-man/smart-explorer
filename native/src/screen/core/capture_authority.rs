//! Shared W/X funding and exact prepared-import registration.
use super::*;
pub(super) struct SharedAuthority(pub(super) Arc<SessionInner>, pub(super) Arc<str>);
impl CaptureAuthority for SharedAuthority {
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant> {
        if plan.is_import() {return Err(invalid("imports require the prepared import authority"));}
        self.fund(plan,None)
    }
    fn register_prepared_source(&self,source:SourceKey)->io::Result<()> {
        if source.session()!=self.1.as_ref() {return Err(invalid("prepared source has another session"));}
        let mut state=self.0.state.lock().map_err(|_|io::Error::other("capture admission poisoned"))?;
        if let Some(entry) = state.prepared.iter_mut().find(|entry| entry.source == source) {
            entry.manual = true;
        } else {
            state.prepared.try_reserve(1).map_err(|_|io::Error::other("prepared source registry allocation failed"))?;
            state.prepared.push(PreparedImportSource {source, manual:true, owners:Vec::new(), connection:None});
        }
        Ok(())
    }
    fn unregister_prepared_source(&self, source: &SourceKey) {
        if source.session() != self.1.as_ref() {return;}
        let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = state.prepared.iter_mut().find(|entry| &entry.source == source) {entry.manual = false;}
        state.prepared.retain(|entry| entry.manual || !entry.owners.is_empty());
    }
    fn own_prepared_source(&self, source: SourceKey) -> io::Result<PreparedSourceOwnerId> {
        if source.session() != self.1.as_ref() {return Err(invalid("prepared owner has another session"));}
        let mut state = self.0.state.lock().map_err(|_|io::Error::other("capture admission poisoned"))?;
        let id = PreparedSourceOwnerId::new(state.next_prepared_owner)?;
        let next = state.next_prepared_owner.checked_add(1).ok_or_else(||invalid("prepared owner IDs exhausted"))?;
        if let Some(entry) = state.prepared.iter_mut().find(|entry| entry.source == source) {
            entry.owners.try_reserve(1).map_err(|_|io::Error::other("prepared owner allocation failed"))?;
            entry.owners.push(id);
        } else {
            let mut owners = Vec::new();
            owners.try_reserve_exact(1).map_err(|_|io::Error::other("prepared owner allocation failed"))?;
            owners.push(id);
            state.prepared.try_reserve(1).map_err(|_|io::Error::other("prepared source registry allocation failed"))?;
            state.prepared.push(PreparedImportSource {source, manual:false, owners, connection:None});
        }
        state.next_prepared_owner = next;
        Ok(id)
    }
    fn release_prepared_source(&self, source: &SourceKey, owner: PreparedSourceOwnerId) {
        if source.session() != self.1.as_ref() {return;}
        let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = state.prepared.iter_mut().find(|entry| &entry.source == source) {
            entry.owners.retain(|id| *id != owner);
        }
        state.prepared.retain(|entry| entry.manual || !entry.owners.is_empty());
    }
    fn begin_import_connection(&self,source:&SourceKey)->io::Result<u64> {
        if source.session() != self.1.as_ref() {return Err(invalid("import connection has another source session"));}
        let mut state=self.0.state.lock().map_err(|_|io::Error::other("capture admission poisoned"))?;
        let index=state.prepared.iter().position(|entry|&entry.source==source).ok_or_else(||invalid("source has no registered preparation"))?;
        let generation=state.next_connection;state.next_connection=generation.checked_add(1).ok_or_else(||invalid("import connection generation exhausted"))?;
        state.prepared[index].connection=Some((generation,0));Ok(generation)
    }
    fn end_import_connection(&self,source:&SourceKey,generation:u64) {
        if source.session() != self.1.as_ref() {return;}
        let mut state=self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry)=state.prepared.iter_mut().find(|entry|&entry.source==source) {
            if entry.connection.is_some_and(|(current,_)|current==generation) {entry.connection=None;}
        }
    }
    fn approve_import(&self,need:super::super::capture_import::CaptureImportNeed)->io::Result<super::super::capture_import::CaptureImportGrant> {
        let plan=need.plan()?;let grant=self.fund(plan,Some(&need))?;
        super::super::capture_import::CaptureImportGrant::from_control_grant(need,grant)
    }
}
impl SharedAuthority {
    fn fund(&self, plan:CapturePlan, import:Option<&super::super::capture_import::CaptureImportNeed>)->io::Result<CaptureGrant> {
        let inner = &self.0;
        if plan.source().session() != self.1.as_ref() || plan.peak_bytes() == 0 {
            return Err(invalid("capture plan has wrong session or no ownership"));
        }
        let mut state = inner.state.lock().map_err(|_| io::Error::other("capture admission poisoned"))?;
        let import_index=if let Some(need)=import {
            let index=state.prepared.iter().position(|entry|&entry.source==need.source()).ok_or_else(||invalid("import source was not prepared"))?;
            let (connection,object)=state.prepared[index].connection.ok_or_else(||invalid("import connection is closed"))?;
            if connection!=need.id().connection() || need.id().object()<=object {return Err(invalid("stale or repeated prepared import"));}
            Some(index)
        } else {None};
        if import.is_none() && state.binding == ContractBinding::Unbound {
            return Err(invalid("capture contract is not bound"));
        }
        if let Some(contract) = plan.contract_binding() {
            if state.binding != ContractBinding::Bound(contract) {
                return Err(invalid("capture plan differs from bound session contract"));
            }
        }
        let known = state.generations.iter().position(|(source, _, _)| source == plan.source());
        if known.is_some_and(|index| plan.generation() < state.generations[index].1) {
            return Err(invalid("stale capture generation"));
        }
        if known.is_none() {
            state.generations.try_reserve(1).map_err(|_| io::Error::other("capture identity allocation failed"))?;
        }
        let owners = known.map_or(Some(1), |index| state.generations[index].2.checked_add(1))
            .ok_or_else(|| invalid("capture source owner count overflow"))?;
        let planned = state.planned.checked_add(plan.peak_bytes()).ok_or_else(|| invalid("capture plans overflow"))?;
        let total = planned.max(inner.working.bytes());
        if plan.peak_bytes() > inner.maximum_total {
            return Err(io::Error::other("capture plan exceeds empty platform capacity ceiling"));
        }
        if total > inner.maximum_total {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                "live capture plans temporarily occupy the platform capacity"));
        }
        state.extra.resize_exact(total - inner.working.bytes())?;
        state.planned = planned;
        if let (Some(index),Some(need))=(import_index,import) {
            state.prepared[index].connection=Some((need.id().connection(),need.id().object()));
        }
        if let Some(index) = known {
            state.generations[index].1 = plan.generation();
            state.generations[index].2 = owners;
        } else {state.generations.push((plan.source().clone(), plan.generation(), owners));}
        Ok(CaptureGrant::new(plan, inner.clone()))
    }
}
impl SessionInner {
    /// Called while the retiring root ledger is held. This changes only its
    /// funded amount; its source/generation owner remains registered until the
    /// root's final release_plan. approve never locks an existing root ledger,
    /// so root -> session -> pool has no reverse session -> existing-root edge.
    pub(in crate::screen::core) fn reduce_plan(&self, previous: u64, remaining: u64) -> io::Result<()> {
        let released = previous.checked_sub(remaining).ok_or_else(|| invalid("capture retirement cannot enlarge a plan"))?;
        let mut state = self.state.lock().map_err(|_| io::Error::other("capture admission poisoned"))?;
        let planned = state.planned.checked_sub(released).ok_or_else(|| invalid("capture retirement exceeds funded plans"))?;
        state.planned = planned;
        state.extra.release_to(planned.saturating_sub(self.working.bytes()));
        Ok(())
    }
    pub(in crate::screen::core) fn release_plan(&self, source: &SourceKey, bytes: u64) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(index) = state.generations.iter().position(|(key, _, _)| key == source) {
            state.generations[index].2 -= 1;
            if state.generations[index].2 == 0 {state.generations.remove(index);}
        }
        state.planned = state.planned.saturating_sub(bytes);
        let extra = state.planned.saturating_sub(self.working.bytes());
        state.extra.release_to(extra);
    }
}
