//! Capture grant identity, construction, and shared ledger ownership.
use std::io;
use std::sync::{Arc, Mutex, Weak};

use super::capture_admission::{CaptureGeneration, SessionInner, SourceKey};
use super::capture_plan::{CapturePlan, OWNER_COUNT};
use super::image_admission::ImageAdmission;
use super::wire::invalid;

#[path = "capture_grant_claims.rs"]
mod claims;
#[path = "capture_grant_slices.rs"]
mod slices;

pub(crate) use slices::{AdmittedResource, CaptureSlice};

struct Usage {
    owners: [u64; OWNER_COUNT],
    total: u64,
    scopes: Vec<Weak<MemberScope>>,
    retired: bool,
    reserved: u64,
}

struct MemberUsage {
    owners: [u64; OWNER_COUNT],
    total: u64,
}

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
    notifications: Mutex<()>,
}

enum Backing {
    Session(Arc<SessionInner>),
    Delegated(DelegatedCaptureCredit),
    LocalImage(Arc<ImageAdmission>),
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
    observer: Option<Arc<dyn Fn([u64; OWNER_COUNT]) + Send + Sync>>,
}

impl DelegatedCaptureCredit {
    pub(crate) fn from_verified_parent(
        source: SourceKey,
        generation: CaptureGeneration,
        total: u64,
        owners: [u64; OWNER_COUNT],
        retirement: impl CaptureCreditLifetime + 'static,
    ) -> io::Result<Self> {
        if total == 0
            || owners.iter().all(|bytes| *bytes == 0)
            || owners.iter().any(|bytes| *bytes > total)
        {
            return Err(invalid("invalid delegated capture limits"));
        }
        Ok(Self {
            source,
            generation,
            total,
            owners,
            _retirement: Box::new(retirement),
            observer: None,
        })
    }

    pub(crate) fn from_verified_parent_observed(
        source: SourceKey,
        generation: CaptureGeneration,
        total: u64,
        owners: [u64; OWNER_COUNT],
        retirement: impl CaptureCreditLifetime + 'static,
        observer: Arc<dyn Fn([u64; OWNER_COUNT]) + Send + Sync>,
    ) -> io::Result<Self> {
        let mut credit = Self::from_verified_parent(source, generation, total, owners, retirement)?;
        credit.observer = Some(observer);
        Ok(credit)
    }
}

/// Cloning a grant shares one promise. It does not create another byte pool.
#[derive(Clone)]
pub(crate) struct CaptureGrant {
    inner: Arc<GrantInner>,
    member: Option<Arc<MemberScope>>,
}

impl CaptureGrant {
    pub(super) fn new(plan: CapturePlan, session: Arc<SessionInner>) -> Self {
        Self::with_backing(plan, Backing::Session(session))
    }

    /// Identity/lifetime compatibility for CapturedImage::grant(). This is
    /// deliberately no capture authority: local copies require ImageRole.
    pub(super) fn for_local_image(
        admission: Arc<ImageAdmission>,
        source: SourceKey,
        generation: CaptureGeneration,
    ) -> Self {
        Self::with_backing(
            CapturePlan::local_identity(source, generation),
            Backing::LocalImage(admission),
        )
    }

    pub(crate) fn image_admission(&self) -> Option<Arc<ImageAdmission>> {
        match &self.inner.backing {
            Backing::LocalImage(admission) => Some(admission.clone()),
            _ => None,
        }
    }

    fn with_backing(plan: CapturePlan, backing: Backing) -> Self {
        let limits = plan.owner_limits();
        let capacity = plan.peak_bytes();
        Self {
            inner: Arc::new(GrantInner {
                plan,
                limits,
                capacity,
                backing,
                lifetimes: Mutex::new(Vec::new()),
                usage: Mutex::new(Usage {
                    owners: [0; OWNER_COUNT],
                    total: 0,
                    scopes: Vec::new(),
                    retired: false,
                    reserved: capacity,
                }),
                notifications: Mutex::new(()),
            }),
            member: None,
        }
    }

    pub(super) fn retain_lifetime(
        &self,
        lifetime: Arc<dyn CaptureCreditLifetime>,
    ) -> io::Result<()> {
        let mut lifetimes = self
            .inner
            .lifetimes
            .lock()
            .map_err(|_| io::Error::other("capture lifetime bookkeeping poisoned"))?;
        lifetimes
            .try_reserve(1)
            .map_err(|_| io::Error::other("capture lifetime bookkeeping allocation failed"))?;
        lifetimes.push(lifetime);
        Ok(())
    }

    /// Consumes exactly one verified credit; no independent memory observation
    /// or W reservation occurs in the worker. Clone shares this local ledger.
    pub(crate) fn from_delegated(
        plan: CapturePlan,
        credit: DelegatedCaptureCredit,
    ) -> io::Result<Self> {
        if plan.source() != &credit.source
            || plan.generation() != credit.generation
            || plan.peak_bytes() != credit.total
            || plan.owner_limits() != credit.owners
        {
            return Err(invalid("delegated credit does not match capture plan"));
        }
        Ok(Self::with_backing(plan, Backing::Delegated(credit)))
    }

    pub(crate) fn source(&self) -> &SourceKey {
        self.plan().source()
    }

    pub(crate) fn generation(&self) -> CaptureGeneration {
        self.plan().generation()
    }

    /// Process-local identity for a weak dispatcher register, never a wire ID.
    pub(crate) fn book_identity(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }

    pub(crate) fn is_member(&self) -> bool {
        self.member.is_some()
    }

    pub(crate) fn allows_images(&self) -> bool {
        !self.inner.plan.is_import()
    }

    pub(crate) fn check_supplement(&self, plan: &CapturePlan) -> io::Result<()> {
        self.check_identity(plan.source(), plan.generation())?;
        if !self.allows_images()
            || self.image_admission().is_some()
            || plan.is_import()
            || plan.peak_bytes() == 0
            || plan.requirements().iter().any(|item| {
                item.source() != self.source()
                    || item.generation() != self.generation()
                    || !matches!(
                        item.owner(),
                        super::capture_plan::CaptureOwner::Readback
                            | super::capture_plan::CaptureOwner::Metadata
                    )
            })
        {
            return Err(invalid(
                "capture supplement must own only this image's cursor bytes",
            ));
        }
        if self.is_retired()? {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "capture supplement's image has retired",
            ));
        }
        Ok(())
    }

    pub(crate) fn plan(&self) -> &CapturePlan {
        self.member
            .as_ref()
            .map_or(&self.inner.plan, |scope| &scope.plan)
    }

    /// A family member receives its own source/generation and local limits,
    /// while all member claims still debit the exact same root ledger.
    pub(crate) fn for_member(
        &self,
        source: &SourceKey,
        generation: CaptureGeneration,
    ) -> io::Result<Self> {
        if self.source() == source && self.generation() == generation {
            return Ok(self.clone());
        }
        if self.member.is_some() || self.image_admission().is_some() {
            return Err(invalid("bound capture grant cannot change its source"));
        }
        let plan = self.inner.plan.for_source(source, generation)?;
        let mut usage = self
            .inner
            .usage
            .lock()
            .map_err(|_| io::Error::other("capture grant poisoned"))?;
        usage.scopes.retain(|scope| scope.strong_count() > 0);
        for scope in usage.scopes.iter().filter_map(Weak::upgrade) {
            if scope.plan.source() == source && scope.plan.generation() == generation {
                return Ok(Self {
                    inner: self.inner.clone(),
                    member: Some(scope),
                });
            }
        }
        usage
            .scopes
            .try_reserve(1)
            .map_err(|_| io::Error::other("capture member bookkeeping allocation failed"))?;
        let limits = plan.owner_limits();
        let capacity = plan.peak_bytes();
        let scope = Arc::new(MemberScope {
            plan,
            limits,
            capacity,
            usage: Mutex::new(MemberUsage {
                owners: [0; OWNER_COUNT],
                total: 0,
            }),
        });
        usage.scopes.push(Arc::downgrade(&scope));
        Ok(Self {
            inner: self.inner.clone(),
            member: Some(scope),
        })
    }

    pub(crate) fn check_identity(
        &self,
        source: &SourceKey,
        generation: CaptureGeneration,
    ) -> io::Result<()> {
        if self.source() != source || self.generation() != generation {
            return Err(invalid("capture grant source or generation mismatch"));
        }
        Ok(())
    }
}
