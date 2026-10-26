//! Local or verified delegated session context, with one final binding.
use super::*;
impl SessionAdmission {
    pub(in crate::screen::core) fn new(session: &str, lease: MemoryLease, maximum_total: u64) -> Self {
        let extra = lease.empty_sibling();
        Self {backing: AdmissionBacking::Local(Arc::new(SessionInner {session: session.into(), maximum_total,
            state: Mutex::new(AdmissionState {binding: ContractBinding::Unbound,
                planned: 0, generations: Vec::new(), extra,prepared:Vec::new(),next_connection:1,next_prepared_owner:1,next_source_session:1}), working: lease}))}
    }
    /// Verified dispatcher context, not a new memory authority. The caller
    /// authenticates the Parent and Accepted identity before construction and
    /// retains its Parent session ticket in lifetime. No local facts, pool or
    /// W lease is created. Each approved grant has its own retirement backing.
    pub(crate) fn from_verified_parent(session: &str, working: u64,
        contract: Option<FrameContract>, authority: Arc<dyn CaptureAuthority>,
        lifetime: impl CaptureCreditLifetime + 'static) -> io::Result<Self> {
        validate_identity(session)?;
        if !(WORKING_IMAGES * 4..=MAX_SESSION_WORKING_BYTES).contains(&working) {
            return Err(invalid("delegated working-memory descriptor is invalid"));
        }
        check_contract(contract, working)?;
        Ok(Self {backing: AdmissionBacking::Delegated(Arc::new(DelegatedSession {
            session: session.into(), working, contract: Mutex::new(ContractBinding::Bound(contract)),
            authority, lifetime: Arc::new(lifetime)}))})
    }
    pub(crate) fn from_verified_parent_unbound(session: &str, working: u64,
        authority: Arc<dyn CaptureAuthority>, lifetime: impl CaptureCreditLifetime + 'static) -> io::Result<Self> {
        validate_identity(session)?;
        if !(WORKING_IMAGES * 4..=MAX_SESSION_WORKING_BYTES).contains(&working) {
            return Err(invalid("delegated working-memory descriptor is invalid"));
        }
        Ok(Self {backing: AdmissionBacking::Delegated(Arc::new(DelegatedSession {
            session: session.into(), working, contract: Mutex::new(ContractBinding::Unbound),
            authority, lifetime: Arc::new(lifetime)}))})
    }
    pub(crate) fn session(&self) -> &str {
        match &self.backing {
            AdmissionBacking::Local(inner) => &inner.session,
            AdmissionBacking::Scoped {session,..} => session,
            AdmissionBacking::Delegated(inner) => &inner.session,
        }
    }
    /// A source replacement gets a fresh identity without acquiring another W.
    /// All plans and imports still use the same atomic bank and final binding.
    pub(crate) fn new_source_session(&self) -> io::Result<Arc<Self>> {
        let inner = match &self.backing {
            AdmissionBacking::Local(inner) | AdmissionBacking::Scoped {inner,..} => inner.clone(),
            AdmissionBacking::Delegated(_) => return Err(io::Error::new(io::ErrorKind::Unsupported,
                "only the Parent may create a source-session scope")),
        };
        let session = {
            let mut state = inner.state.lock().map_err(|_|io::Error::other("capture admission poisoned"))?;
            let id = state.next_source_session;
            state.next_source_session = id.checked_add(1).ok_or_else(||invalid("source session identities exhausted"))?;
            format!("{}/source:{id}",inner.session)
        };
        Ok(Arc::new(Self {backing: AdmissionBacking::Scoped {inner, session:session.into()}}))
    }
    /// Called only by the authorized metadata prepare boundary, before the
    /// platform opens its connection. This grants no image or source-opening right.
    pub(crate) fn register_prepared_source(&self,source:SourceKey)->io::Result<()> {
        if source.session()!=self.session() {return Err(invalid("prepared source belongs to another session"));}
        self.authority().register_prepared_source(source)
    }
    pub(crate) fn import_connection(&self,source:&SourceKey)->io::Result<super::super::capture_import::CaptureImportConnection> {
        if source.session()!=self.session() {return Err(invalid("prepared import source belongs to another session"));}
        let authority=self.authority();let generation=authority.begin_import_connection(source)?;
        super::super::capture_import::CaptureImportConnection::new(source.clone(),generation,authority)
    }
    pub(crate) fn cancel_pending_capture(&self,source:&SourceKey) {self.authority().cancel_pending(source);}
    pub(crate) fn working_bytes(&self) -> u64 {
        match &self.backing {
            AdmissionBacking::Local(inner) | AdmissionBacking::Scoped {inner,..} => inner.working.bytes(),
            AdmissionBacking::Delegated(inner) => inner.working,
        }
    }
    pub(crate) fn bind_contract(&self, contract: Option<FrameContract>) -> io::Result<()> {
        check_contract(contract, self.working_bytes())?;
        let inner = match &self.backing {
            AdmissionBacking::Local(inner) | AdmissionBacking::Scoped {inner,..} => inner,
            AdmissionBacking::Delegated(inner) => {
                let mut binding = inner.contract.lock().map_err(|_| io::Error::other("delegated session binding poisoned"))?;
                return match *binding {
                    ContractBinding::Unbound => {*binding = ContractBinding::Bound(contract); Ok(())}
                    ContractBinding::Bound(previous) if previous == contract => Ok(()),
                    _ => Err(invalid("delegated session contract already bound differently")),
                };
            }
        };
        let mut state = inner.state.lock().map_err(|_| io::Error::other("capture admission poisoned"))?;
        match state.binding {
            ContractBinding::Unbound => {state.binding = ContractBinding::Bound(contract); Ok(())}
            ContractBinding::Bound(bound) if bound == contract => Ok(()),
            ContractBinding::Bound(_) => Err(invalid("capture contract already bound differently")),
        }
    }
    pub(crate) fn authority(&self) -> Arc<dyn CaptureAuthority> {
        match &self.backing {
            AdmissionBacking::Local(inner) | AdmissionBacking::Scoped {inner,..} =>
                Arc::new(SharedAuthority(inner.clone(), self.session().into())),
            AdmissionBacking::Delegated(inner) => inner.clone(),
        }
    }
    pub(crate) fn additional_bytes(&self) -> io::Result<u64> {
        match &self.backing {
            AdmissionBacking::Local(inner) | AdmissionBacking::Scoped {inner,..} => Ok(inner.state.lock()
                .map_err(|_| io::Error::other("capture admission poisoned"))?.extra.bytes()),
            AdmissionBacking::Delegated(_) => Err(io::Error::new(io::ErrorKind::Unsupported,
                "additional capture memory is accounted only by Parent")),
        }
    }
}
fn check_contract(contract: Option<FrameContract>, working: u64) -> io::Result<()> {
    if let Some(contract) = contract {
        contract.validate()?;
        if contract.limits.working_bytes > working {
            return Err(invalid("capture contract exceeds admitted working memory"));
        }
    } else if working>MAX_WORKING_BYTES {
        return Err(invalid("legacy capture exceeds its allocation class"));
    } else {
        super::super::frame_budget::legacy_limits(working,None)?;
    }
    Ok(())
}
impl CaptureAuthority for DelegatedSession {
    fn register_prepared_source(&self,source:SourceKey)->io::Result<()> {
        if source.session()!=self.session.as_ref() {return Err(invalid("delegated prepare has another session"));}
        self.authority.register_prepared_source(source)
    }
    fn unregister_prepared_source(&self, source: &SourceKey) {
        if source.session() == self.session.as_ref() {
            self.authority.unregister_prepared_source(source);
        }
    }
    fn own_prepared_source(&self, source: SourceKey) -> io::Result<PreparedSourceOwnerId> {
        if source.session() != self.session.as_ref() {return Err(invalid("prepared owner has another session"));}
        self.authority.own_prepared_source(source)
    }
    fn release_prepared_source(&self, source: &SourceKey, owner: PreparedSourceOwnerId) {
        if source.session() == self.session.as_ref() {self.authority.release_prepared_source(source, owner);}
    }
    fn begin_import_connection(&self,source:&SourceKey)->io::Result<u64> {
        if source.session()!=self.session.as_ref() {return Err(invalid("delegated import has another session"));}
        self.authority.begin_import_connection(source)
    }
    fn end_import_connection(&self,source:&SourceKey,generation:u64) {self.authority.end_import_connection(source,generation);}
    fn cancel_pending(&self,source:&SourceKey) {self.authority.cancel_pending(source);}
    fn approve_import(&self,need:super::super::capture_import::CaptureImportNeed)->io::Result<super::super::capture_import::CaptureImportGrant> {
        if need.source().session()!=self.session.as_ref() {return Err(invalid("delegated import has another session"));}
        let grant=self.authority.approve_import(need.clone())?;
        grant.check_identity(need.source(),&need.id())?;grant.retain_lifetime(self.lifetime.clone())?;Ok(grant)
    }
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant> {
        self.approve_plan(plan, None)
    }
    fn approve_supplement(&self, image: &CaptureGrant, plan: CapturePlan) -> io::Result<CaptureGrant> {
        image.check_supplement(&plan)?;
        self.approve_plan(plan, Some(image))
    }
}
impl DelegatedSession {
    fn approve_plan(&self, plan: CapturePlan, image: Option<&CaptureGrant>) -> io::Result<CaptureGrant> {
        let binding = *self.contract.lock().map_err(|_| io::Error::other("delegated session binding poisoned"))?;
        let ContractBinding::Bound(contract) = binding else {return Err(invalid("delegated capture contract is not bound"));};
        if plan.source().session() != self.session.as_ref()
            || plan.contract_binding().is_some_and(|planned| planned != contract) {
            return Err(invalid("delegated capture plan has another session or contract"));
        }
        let source = plan.source().clone();
        let generation = plan.generation();
        let total = plan.peak_bytes();
        let owners = plan.owner_limits();
        let grant = if let Some(image) = image {self.authority.approve_supplement(image, plan)?}
            else {self.authority.approve(plan)?};
        grant.check_identity(&source, generation)?;
        if grant.plan().peak_bytes() != total || grant.plan().owner_limits() != owners {
            return Err(invalid("Parent capture reply differs from requested ownership"));
        }
        grant.retain_lifetime(self.lifetime.clone())?;
        Ok(grant)
    }
}
