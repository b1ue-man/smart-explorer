//! One session's unchanged W and its exact additional host ownership X.
use super::capture_grant::CaptureCreditLifetime;
use super::frame_budget::{
    FrameContract, MAX_SESSION_WORKING_BYTES, MAX_WORKING_BYTES, WORKING_IMAGES,
};
use super::frame_memory::MemoryLease;
use super::wire::invalid;
use std::io;
use std::sync::{Arc, Mutex};

pub(crate) use super::capture_grant::{CaptureGrant, CaptureSlice};
pub(crate) use super::capture_plan::{CaptureOwner, CapturePlan};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceKey {
    session: Arc<str>,
    source: Arc<str>,
}
impl SourceKey {
    pub(crate) fn new(session: &str, source: &str) -> io::Result<Self> {
        validate_identity(session)?;
        validate_identity(source)?;
        Ok(Self {
            session: session.into(),
            source: source.into(),
        })
    }
    pub(crate) fn session(&self) -> &str {
        &self.session
    }
    pub(crate) fn source(&self) -> &str {
        &self.source
    }
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
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
    pub(crate) fn value(self) -> u64 {
        self.0
    }
    pub(crate) fn next(self) -> io::Result<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| invalid("capture generation exhausted"))
    }
}
/// One live preparation/source owner; never an image-allocation capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PreparedSourceOwnerId(u64);
impl PreparedSourceOwnerId {
    pub(crate) fn new(value: u64) -> io::Result<Self> {
        if value == 0 {
            return Err(invalid("zero prepared source owner identity"));
        }
        Ok(Self(value))
    }
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}
pub(crate) trait CaptureAuthority: Send + Sync {
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant>;
    /// Additional cursor ownership, never a replacement image/output plan.
    fn approve_supplement(
        &self,
        image: &CaptureGrant,
        plan: CapturePlan,
    ) -> io::Result<CaptureGrant> {
        image.check_supplement(&plan)?;
        self.approve(plan)
    }
    fn register_prepared_source(&self, _source: SourceKey) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "authority has no prepared import scope",
        ))
    }
    fn unregister_prepared_source(&self, _source: &SourceKey) {}
    fn own_prepared_source(&self, _source: SourceKey) -> io::Result<PreparedSourceOwnerId> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "authority has no prepared source owner",
        ))
    }
    fn release_prepared_source(&self, _source: &SourceKey, _owner: PreparedSourceOwnerId) {}
    fn begin_import_connection(&self, _source: &SourceKey) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "authority has no prepared import connection",
        ))
    }
    fn end_import_connection(&self, _source: &SourceKey, _generation: u64) {}
    fn cancel_pending(&self, _source: &SourceKey) {}
    fn approve_import(
        &self,
        _need: super::capture_import::CaptureImportNeed,
    ) -> io::Result<super::capture_import::CaptureImportGrant> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "authority has no prepared import admission",
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContractBinding {
    Unbound,
    Bound(Option<FrameContract>),
}
struct AdmissionState {
    binding: ContractBinding,
    planned: u64,
    generations: Vec<(SourceKey, CaptureGeneration, u64)>,
    extra: MemoryLease,
    prepared: Vec<PreparedImportSource>,
    next_connection: u64,
    next_prepared_owner: u64,
    next_source_session: u64,
}
struct PreparedImportSource {
    source: SourceKey,
    manual: bool,
    owners: Vec<PreparedSourceOwnerId>,
    connection: Option<(u64, u64)>,
}
pub(super) struct SessionInner {
    session: Arc<str>,
    maximum_total: u64,
    state: Mutex<AdmissionState>,
    // X and every grant are gone before the final W lease is released.
    working: MemoryLease,
}
enum AdmissionBacking {
    Local(Arc<SessionInner>),
    Scoped {
        inner: Arc<SessionInner>,
        session: Arc<str>,
    },
    Delegated(Arc<DelegatedSession>),
}
struct DelegatedSession {
    session: Arc<str>,
    working: u64,
    contract: Mutex<ContractBinding>,
    authority: Arc<dyn CaptureAuthority>,
    lifetime: Arc<dyn CaptureCreditLifetime>,
}
pub(crate) struct SessionAdmission {
    backing: AdmissionBacking,
}
#[path = "capture_session_context.rs"]
mod context;
#[path = "capture_authority.rs"]
mod local_authority;
use local_authority::SharedAuthority;
