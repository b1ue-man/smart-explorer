//! One session's unchanged W and its exact additional host ownership X.
use std::io;
use std::sync::{Arc, Mutex};
use super::frame_budget::{FrameContract, MAX_WORKING_BYTES, MAX_SESSION_WORKING_BYTES, WORKING_IMAGES};
use super::frame_memory::MemoryLease;
use super::capture_grant::CaptureCreditLifetime;
use super::wire::invalid;

pub(crate) use super::capture_grant::{CaptureGrant, CaptureSlice};
pub(crate) use super::capture_plan::{CaptureOwner, CapturePlan};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceKey {session: Arc<str>, source: Arc<str>}
impl SourceKey {
    pub(crate) fn new(session: &str, source: &str) -> io::Result<Self> {
        validate_identity(session)?;
        validate_identity(source)?;
        Ok(Self {session: session.into(), source: source.into()})
    }
    pub(crate) fn session(&self) -> &str {&self.session}
    pub(crate) fn source(&self) -> &str {&self.source}
}
pub(super) fn validate_identity(value: &str) -> io::Result<()> {
    if value.is_empty() || value.contains('\0') {
        return Err(invalid("invalid capture source/session identity"));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CaptureGeneration(u64);
impl CaptureGeneration {
    pub(crate) const fn new(value: u64) -> Self {Self(value)}
    pub(crate) fn value(self) -> u64 {self.0}
    pub(crate) fn next(self) -> io::Result<Self> {
        self.0.checked_add(1).map(Self).ok_or_else(|| invalid("capture generation exhausted"))
    }
}
pub(crate) trait CaptureAuthority: Send + Sync {
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContractBinding {Unbound, Bound(Option<FrameContract>)}
struct AdmissionState {
    binding: ContractBinding,
    planned: u64,
    generations: Vec<(SourceKey, CaptureGeneration, u64)>,
    extra: MemoryLease,
}
pub(super) struct SessionInner {
    session: Arc<str>,
    maximum_total: u64,
    state: Mutex<AdmissionState>,
    // X and every grant are gone before the final W lease is released.
    working: MemoryLease,
}
enum AdmissionBacking {Local(Arc<SessionInner>), Delegated(Arc<DelegatedSession>)}
struct DelegatedSession {
    session: Arc<str>,
    working: u64,
    contract: Option<FrameContract>,
    authority: Arc<dyn CaptureAuthority>,
    lifetime: Arc<dyn CaptureCreditLifetime>,
}
pub(crate) struct SessionAdmission {backing: AdmissionBacking}
impl SessionAdmission {
    pub(super) fn new(session: &str, lease: MemoryLease, maximum_total: u64) -> Self {
        let extra = lease.empty_sibling();
        Self {backing: AdmissionBacking::Local(Arc::new(SessionInner {session: session.into(), maximum_total,
            state: Mutex::new(AdmissionState {binding: ContractBinding::Unbound,
                planned: 0, generations: Vec::new(), extra}), working: lease}))}
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
            session: session.into(), working, contract, authority, lifetime: Arc::new(lifetime)}))})
    }
    pub(crate) fn session(&self) -> &str {
        match &self.backing {
            AdmissionBacking::Local(inner) => &inner.session,
            AdmissionBacking::Delegated(inner) => &inner.session,
        }
    }
    pub(crate) fn working_bytes(&self) -> u64 {
        match &self.backing {
            AdmissionBacking::Local(inner) => inner.working.bytes(),
            AdmissionBacking::Delegated(inner) => inner.working,
        }
    }
    pub(crate) fn bind_contract(&self, contract: Option<FrameContract>) -> io::Result<()> {
        check_contract(contract, self.working_bytes())?;
        let inner = match &self.backing {
            AdmissionBacking::Local(inner) => inner,
            AdmissionBacking::Delegated(inner) => return if inner.contract == contract {Ok(())}
                else {Err(invalid("delegated session contract already bound differently"))},
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
            AdmissionBacking::Local(inner) => Arc::new(SharedAuthority(inner.clone())),
            AdmissionBacking::Delegated(inner) => inner.clone(),
        }
    }
    pub(crate) fn additional_bytes(&self) -> io::Result<u64> {
        match &self.backing {
            AdmissionBacking::Local(inner) => Ok(inner.state.lock()
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
    } else if working != MAX_WORKING_BYTES {
        return Err(io::Error::other("insufficient memory for legacy screen session"));
    }
    Ok(())
}
impl CaptureAuthority for DelegatedSession {
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant> {
        if plan.source().session() != self.session.as_ref()
            || plan.contract_binding().is_some_and(|contract| contract != self.contract) {
            return Err(invalid("delegated capture plan has another session or contract"));
        }
        let source = plan.source().clone();
        let generation = plan.generation();
        let total = plan.peak_bytes();
        let owners = plan.owner_limits();
        let grant = self.authority.approve(plan)?;
        grant.check_identity(&source, generation)?;
        if grant.plan().peak_bytes() != total || grant.plan().owner_limits() != owners {
            return Err(invalid("Parent capture reply differs from requested ownership"));
        }
        grant.retain_lifetime(self.lifetime.clone())?;
        Ok(grant)
    }
}
struct SharedAuthority(Arc<SessionInner>);
impl CaptureAuthority for SharedAuthority {
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant> {
        let inner = &self.0;
        if plan.source().session() != inner.session.as_ref() || plan.peak_bytes() == 0 {
            return Err(invalid("capture plan has wrong session or no ownership"));
        }
        let mut state = inner.state.lock().map_err(|_| io::Error::other("capture admission poisoned"))?;
        if state.binding == ContractBinding::Unbound {
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
        if total > inner.maximum_total {return Err(io::Error::other("capture exceeds platform capacity ceiling"));}
        state.extra.resize_exact(total - inner.working.bytes())?;
        state.planned = planned;
        if let Some(index) = known {
            state.generations[index].1 = plan.generation();
            state.generations[index].2 = owners;
        } else {state.generations.push((plan.source().clone(), plan.generation(), owners));}
        Ok(CaptureGrant::new(plan, inner.clone()))
    }
}
impl SessionInner {
    pub(super) fn release_plan(&self, source: &SourceKey, bytes: u64) {
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
