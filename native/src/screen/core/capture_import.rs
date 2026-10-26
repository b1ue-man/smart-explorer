//! Exact prepared-source imports, independent of image/reference generations.
//! No FD, native pointer, geometry guess, frame binding or pixel constructor.
use super::capture_admission::{
    CaptureAuthority, CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, CaptureSlice,
    SourceKey,
};
use super::capture_plan::CapturePhases;
use super::wire::invalid;
use std::{io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CaptureImportId {
    connection: u64,
    object: u64,
}
impl CaptureImportId {
    pub(crate) fn new(connection: u64, object: u64) -> io::Result<Self> {
        if connection == 0 || object == 0 {
            return Err(invalid("import connection and object IDs must be positive"));
        }
        Ok(Self { connection, object })
    }
    pub(crate) fn connection(self) -> u64 {
        self.connection
    }
    pub(crate) fn object(self) -> u64 {
        self.object
    }
}
#[derive(Clone, Debug)]
pub(crate) struct CaptureImportNeed {
    source: SourceKey,
    id: CaptureImportId,
    mapped: u64,
    metadata: u64,
}
impl CaptureImportNeed {
    pub(crate) fn new(
        source: SourceKey,
        id: CaptureImportId,
        mapped_bytes: u64,
        metadata_bytes: u64,
    ) -> io::Result<Self> {
        if mapped_bytes == 0 || mapped_bytes.checked_add(metadata_bytes).is_none() {
            return Err(invalid("invalid prepared import byte requirement"));
        }
        Ok(Self {
            source,
            id,
            mapped: mapped_bytes,
            metadata: metadata_bytes,
        })
    }
    pub(crate) fn source(&self) -> &SourceKey {
        &self.source
    }
    pub(crate) fn id(&self) -> CaptureImportId {
        self.id
    }
    pub(crate) fn mapped_bytes(&self) -> u64 {
        self.mapped
    }
    pub(crate) fn metadata_bytes(&self) -> u64 {
        self.metadata
    }
    pub(crate) fn control_plan(&self) -> io::Result<CapturePlan> {
        self.plan()
    }
    pub(super) fn plan(&self) -> io::Result<CapturePlan> {
        let ledger = SourceKey::new(
            self.source.session(),
            &format!(
                "prepared-import[{}:{}]:{}",
                self.id.connection,
                self.id.object,
                self.source.source()
            ),
        )?;
        let mut plan = CapturePlan::new(ledger, CaptureGeneration::new(self.id.connection));
        plan.add_bytes(
            CaptureOwner::NativeSurface,
            self.mapped,
            1,
            CapturePhases::ALL,
        )?;
        if self.metadata != 0 {
            plan.add_bytes(CaptureOwner::Metadata, self.metadata, 1, CapturePhases::ALL)?;
        }
        plan.mark_import()?;
        Ok(plan)
    }
}
pub(crate) struct CaptureImportConnection {
    source: SourceKey,
    generation: u64,
    authority: Arc<dyn CaptureAuthority>,
}
impl CaptureImportConnection {
    pub(super) fn new(
        source: SourceKey,
        generation: u64,
        authority: Arc<dyn CaptureAuthority>,
    ) -> io::Result<Self> {
        CaptureImportId::new(generation, 1)?;
        Ok(Self {
            source,
            generation,
            authority,
        })
    }
    pub(crate) fn source(&self) -> &SourceKey {
        &self.source
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn id(&self, object: u64) -> io::Result<CaptureImportId> {
        CaptureImportId::new(self.generation, object)
    }
}
impl Drop for CaptureImportConnection {
    fn drop(&mut self) {
        self.authority
            .end_import_connection(&self.source, self.generation);
    }
}
pub(crate) struct CaptureImportGrant {
    need: CaptureImportNeed,
    grant: CaptureGrant,
}
impl CaptureImportGrant {
    pub(crate) fn from_control_grant(
        need: CaptureImportNeed,
        grant: CaptureGrant,
    ) -> io::Result<Self> {
        let expected = need.plan()?;
        grant.check_identity(expected.source(), expected.generation())?;
        if !grant.plan().is_import()
            || grant.plan().peak_bytes() != expected.peak_bytes()
            || grant.plan().owner_limits() != expected.owner_limits()
        {
            return Err(invalid("prepared import credit differs from exact need"));
        }
        Ok(Self { need, grant })
    }
    pub(crate) fn check_identity(
        &self,
        source: &SourceKey,
        import: &CaptureImportId,
    ) -> io::Result<()> {
        if self.need.source() != source || &self.need.id != import {
            return Err(invalid(
                "prepared import source or connection identity mismatch",
            ));
        }
        Ok(())
    }
    pub(crate) fn claim_mapping(&self) -> io::Result<CaptureSlice> {
        self.grant
            .claim(CaptureOwner::NativeSurface, self.need.mapped)
    }
    pub(crate) fn claim_metadata(&self, bytes: u64) -> io::Result<CaptureSlice> {
        if bytes > self.need.metadata {
            return Err(invalid(
                "prepared import metadata exceeds its prior promise",
            ));
        }
        self.grant.claim(CaptureOwner::Metadata, bytes)
    }
    /// Dispatcher lifecycle only. Purpose remains Import in this shared book;
    /// ImageBuilder/AdmittedBytes reject it even through Slice::grant().
    pub(crate) fn control_grant(&self) -> CaptureGrant {
        self.grant.clone()
    }
    pub(crate) fn retire(&self) -> io::Result<()> {
        self.grant.retire()
    }
    pub(super) fn retain_lifetime(
        &self,
        lifetime: Arc<dyn super::capture_grant::CaptureCreditLifetime>,
    ) -> io::Result<()> {
        self.grant.retain_lifetime(lifetime)
    }
}
