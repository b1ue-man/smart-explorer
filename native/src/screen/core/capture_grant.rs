//! Atomic per-class and whole-grant subdivision. Slices cannot be cloned.
use std::io;
use std::sync::{Arc, Mutex, Weak};
use super::capture_admission::{CaptureGeneration, SessionInner, SourceKey};
use super::capture_plan::{CaptureOwner, CapturePlan, OWNER_COUNT};
use super::wire::invalid;

struct Usage {
    owners: [u64; OWNER_COUNT],
    total: u64,
    scopes: Vec<Weak<MemberScope>>,
}
struct MemberUsage {owners: [u64; OWNER_COUNT], total: u64}
struct MemberScope {
    plan: CapturePlan,
    limits: [u64; OWNER_COUNT],
    capacity: u64,
    usage: Mutex<MemberUsage>,
}
struct GrantInner {
    plan: CapturePlan,
    limits: [u64; OWNER_COUNT],
    capacity: u64,
    usage: Mutex<Usage>,
    backing: Backing,
    // Grant retirement precedes release of its surrounding session context.
    lifetimes: Mutex<Vec<Arc<dyn CaptureCreditLifetime>>>,
}
enum Backing {Session(Arc<SessionInner>), Delegated(DelegatedCaptureCredit)}
impl Drop for GrantInner {
    fn drop(&mut self) {
        if let Backing::Session(session) = &self.backing {
            session.release_plan(self.plan.source(), self.plan.peak_bytes());
        }
    }
}
/// Marker for an OS dispatcher's retirement owner. Its Drop runs only after
/// the final local grant/slice. The dispatcher, not this core, authenticates
/// the parent and keeps the real parent promise until acknowledged retirement
/// or confirmed process exit. The owner may retain a callback and its channel.
pub(crate) trait CaptureCreditLifetime: Send + Sync {}
impl<T: Send + Sync> CaptureCreditLifetime for T {}

/// Crate-internal trust boundary, deliberately neither Clone nor Serde.
/// Construction says the dispatcher has verified channel, grant ID, source,
/// generation, and disjoint parent/worker limits. It performs no authentication.
pub(crate) struct DelegatedCaptureCredit {
    source: SourceKey,
    generation: CaptureGeneration,
    total: u64,
    owners: [u64; OWNER_COUNT],
    _retirement: Box<dyn CaptureCreditLifetime>,
}
impl DelegatedCaptureCredit {
    pub(crate) fn from_verified_parent(source: SourceKey, generation: CaptureGeneration,
        total: u64, owners: [u64; OWNER_COUNT], retirement: impl CaptureCreditLifetime + 'static)
        -> io::Result<Self> {
        if total == 0 || owners.iter().all(|bytes| *bytes == 0) || owners.iter().any(|bytes| *bytes > total) {
            return Err(invalid("invalid delegated capture limits"));
        }
        Ok(Self {source, generation, total, owners, _retirement: Box::new(retirement)})
    }
}
/// Cloning a grant shares one promise. It does not create another byte pool.
#[derive(Clone)]
pub(crate) struct CaptureGrant {inner: Arc<GrantInner>, member: Option<Arc<MemberScope>>}
pub(crate) struct CaptureSlice {
    grant: Option<Arc<GrantInner>>,
    member: Option<Arc<MemberScope>>,
    owner: CaptureOwner,
    bytes: u64,
}
/// Holds native resources in the same destruction order as pixel storage.
/// The closure passed to allocate runs only after the slice was acquired.
pub(crate) struct AdmittedResource<T> {resource: T, _slice: CaptureSlice}
impl<T> AdmittedResource<T> {
    pub(crate) fn resource(&self) -> &T {&self.resource}
}
impl CaptureGrant {
    pub(super) fn new(plan: CapturePlan, session: Arc<SessionInner>) -> Self {
        Self::with_backing(plan, Backing::Session(session))
    }
    fn with_backing(plan: CapturePlan, backing: Backing) -> Self {
        let limits = plan.owner_limits();
        let capacity = plan.peak_bytes();
        Self {inner: Arc::new(GrantInner {plan, limits, capacity, backing, lifetimes: Mutex::new(Vec::new()),
            usage: Mutex::new(Usage {owners: [0; OWNER_COUNT], total: 0, scopes: Vec::new()})}), member: None}
    }
    pub(super) fn retain_lifetime(&self, lifetime: Arc<dyn CaptureCreditLifetime>) -> io::Result<()> {
        let mut lifetimes = self.inner.lifetimes.lock().map_err(|_| io::Error::other("capture lifetime bookkeeping poisoned"))?;
        lifetimes.try_reserve(1).map_err(|_| io::Error::other("capture lifetime bookkeeping allocation failed"))?;
        lifetimes.push(lifetime);
        Ok(())
    }
    /// Consumes exactly one verified credit; no independent memory observation
    /// or W reservation occurs in the worker. Clone shares this local ledger.
    pub(crate) fn from_delegated(plan: CapturePlan, credit: DelegatedCaptureCredit) -> io::Result<Self> {
        if plan.source() != &credit.source || plan.generation() != credit.generation
            || plan.peak_bytes() != credit.total || plan.owner_limits() != credit.owners {
            return Err(invalid("delegated credit does not match capture plan"));
        }
        Ok(Self::with_backing(plan, Backing::Delegated(credit)))
    }
    pub(crate) fn source(&self) -> &SourceKey {self.plan().source()}
    pub(crate) fn generation(&self) -> CaptureGeneration {self.plan().generation()}
    pub(crate) fn plan(&self) -> &CapturePlan {
        self.member.as_ref().map_or(&self.inner.plan, |scope| &scope.plan)
    }
    /// A family member receives its own source/generation and local limits,
    /// while all member claims still debit the exact same root ledger.
    pub(crate) fn for_member(&self, source: &SourceKey, generation: CaptureGeneration) -> io::Result<Self> {
        if self.source() == source && self.generation() == generation {return Ok(self.clone());}
        if self.member.is_some() {return Err(invalid("member capture grant cannot change its source"));}
        let plan = self.inner.plan.for_source(source, generation)?;
        let mut usage = self.inner.usage.lock().map_err(|_| io::Error::other("capture grant poisoned"))?;
        usage.scopes.retain(|scope| scope.strong_count() > 0);
        for scope in usage.scopes.iter().filter_map(Weak::upgrade) {
            if scope.plan.source() == source && scope.plan.generation() == generation {
                return Ok(Self {inner: self.inner.clone(), member: Some(scope)});
            }
        }
        usage.scopes.try_reserve(1).map_err(|_| io::Error::other("capture member bookkeeping allocation failed"))?;
        let limits = plan.owner_limits();
        let capacity = plan.peak_bytes();
        let scope = Arc::new(MemberScope {plan, limits, capacity,
            usage: Mutex::new(MemberUsage {owners: [0; OWNER_COUNT], total: 0})});
        usage.scopes.push(Arc::downgrade(&scope));
        Ok(Self {inner: self.inner.clone(), member: Some(scope)})
    }
    pub(crate) fn check_identity(&self, source: &SourceKey, generation: CaptureGeneration) -> io::Result<()> {
        if self.source() != source || self.generation() != generation {
            return Err(invalid("capture grant source or generation mismatch"));
        }
        Ok(())
    }
    pub(crate) fn claim(&self, owner: CaptureOwner, bytes: u64) -> io::Result<CaptureSlice> {
        let mut usage = self.inner.usage.lock().map_err(|_| io::Error::other("capture grant poisoned"))?;
        let owner_total = usage.owners[owner as usize].checked_add(bytes)
            .ok_or_else(|| invalid("capture owner claim overflow"))?;
        let total = usage.total.checked_add(bytes).ok_or_else(|| invalid("capture total claim overflow"))?;
        if owner_total > self.inner.limits[owner as usize] || total > self.inner.capacity {
            return Err(io::Error::other("capture slice exceeds admitted ownership"));
        }
        if let Some(member) = &self.member {
            let mut local = member.usage.lock().map_err(|_| io::Error::other("capture member grant poisoned"))?;
            let local_owner = local.owners[owner as usize].checked_add(bytes)
                .ok_or_else(|| invalid("capture member owner overflow"))?;
            let local_total = local.total.checked_add(bytes).ok_or_else(|| invalid("capture member total overflow"))?;
            if local_owner > member.limits[owner as usize] || local_total > member.capacity {
                return Err(io::Error::other("capture slice exceeds bound member ownership"));
            }
            local.owners[owner as usize] = local_owner;
            local.total = local_total;
        }
        usage.owners[owner as usize] = owner_total;
        usage.total = total;
        // Each non-Clone value owns a distinct logical credit. Pixel buffers
        // are separate allocations, so imposing arena contiguity would invent
        // fragmentation that the product's allocator does not have.
        Ok(CaptureSlice {grant: Some(self.inner.clone()), member: self.member.clone(), owner, bytes})
    }
    pub(crate) fn live_bytes(&self) -> io::Result<u64> {
        Ok(self.inner.usage.lock().map_err(|_| io::Error::other("capture grant poisoned"))?.total)
    }
}
impl CaptureSlice {
    pub(crate) fn bytes(&self) -> u64 {self.bytes}
    pub(crate) fn owner(&self) -> CaptureOwner {self.owner}
    pub(crate) fn grant(&self) -> io::Result<CaptureGrant> {
        let inner = self.grant.as_ref().ok_or_else(|| invalid("capture slice already consumed"))?;
        Ok(CaptureGrant {inner: inner.clone(), member: self.member.clone()})
    }
    pub(crate) fn allocate<T>(self, create: impl FnOnce() -> io::Result<T>) -> io::Result<AdmittedResource<T>> {
        let resource = create()?;
        Ok(AdmittedResource {resource, _slice: self})
    }
    pub(crate) fn belongs_to(&self, grant: &CaptureGrant) -> bool {
        self.grant.as_ref().is_some_and(|inner| Arc::ptr_eq(inner, &grant.inner))
            && same_member(&self.member, &grant.member)
    }
    pub(crate) fn same_grant(&self, other: &Self) -> bool {
        match (&self.grant, &other.grant) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) && same_member(&self.member, &other.member),
            _ => false,
        }
    }
    pub(crate) fn split(mut self, first_bytes: u64) -> io::Result<(Self, Self)> {
        if first_bytes > self.bytes() {return Err(invalid("capture split exceeds slice"));}
        let first = Self {grant: self.grant.clone(), member: self.member.clone(), owner: self.owner, bytes: first_bytes};
        let second = Self {grant: self.grant.take(), member: self.member.take(), owner: self.owner, bytes: self.bytes - first_bytes};
        Ok((first, second))
    }
}
impl Drop for CaptureSlice {
    fn drop(&mut self) {
        let Some(grant) = self.grant.take() else {return;};
        let mut usage = grant.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        usage.total -= self.bytes();
        usage.owners[self.owner as usize] -= self.bytes();
        if let Some(member) = &self.member {
            let mut local = member.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            local.total -= self.bytes();
            local.owners[self.owner as usize] -= self.bytes();
        }
    }
}
fn same_member(a: &Option<Arc<MemberScope>>, b: &Option<Arc<MemberScope>>) -> bool {
    match (a, b) {(None, None) => true, (Some(a), Some(b)) => Arc::ptr_eq(a, b), _ => false}
}
