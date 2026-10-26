//! Same current ScreenView admission as the existing windows/icon metadata path.
use std::io;
use super::host_server::HostContext;
use super::family_protocol::{owner, FamilyListing};
use crate::share::access_grant::Capability;
use crate::share::relation_proof::ProvenPrincipal;

pub(crate) fn list(context: &HostContext, principal: &ProvenPrincipal) -> io::Result<FamilyListing> {
    let owner = owner(principal)?;
    let mut result = None;
    let allowed = context.authority.admit(principal, &[Capability::ScreenView], crate::share::core_now_secs(),
        &mut || { result = Some(context.backend.family_windows(&owner)); });
    if !allowed { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "family_list_permission_revoked")); }
    result.unwrap_or_else(|| Err(io::Error::new(io::ErrorKind::PermissionDenied, "family_list_not_admitted")))
}

pub(crate) fn admitted(context: &HostContext, principal: &ProvenPrincipal, control: bool) -> bool {
    let needs = if control { &[Capability::ScreenView,Capability::ScreenControl][..] }
        else { &[Capability::ScreenView][..] };
    context.authority.admit(principal,needs,crate::share::core_now_secs(),&mut || {})
}

pub(crate) fn open(context: &HostContext, principal: &ProvenPrincipal,
    source: &super::window_list::SourceId, wanted: Option<super::pixel_convert::FrameSize>, control: bool)
    -> Result<Box<dyn super::host_backend::ScreenSource>,super::host_backend::SourceRefusal> {
    let refusal=|error:io::Error|super::host_backend::SourceRefusal::new(&error.to_string());
    if !admitted(context,principal,control) {
        return Err(super::host_backend::SourceRefusal::new("source_permission_revoked"));
    }
    context.backend.open_session_source(&owner(principal).map_err(refusal)?,
        &super::family_protocol::token().map_err(refusal)?,source,wanted,control)
}
pub(crate) fn prepare(context:&HostContext,principal:&ProvenPrincipal,source:&super::window_list::SourceId,
    wanted:Option<super::pixel_convert::FrameSize>,control:bool,
    admission:std::sync::Arc<super::capture_admission::SessionAdmission>)
    ->Result<Box<dyn super::capture_source_contract::PreparedSource>,super::host_backend::SourceRefusal> {
    let refusal=|error:io::Error|super::host_backend::SourceRefusal::new(&error.to_string());
    if !admitted(context,principal,control) {return Err(super::host_backend::SourceRefusal::new("source_permission_revoked"));}
    let prepared=context.backend.prepare_session_source(&owner(principal).map_err(refusal)?,admission.session(),source,wanted,control,admission.clone())?;
    super::host_admission::registered(prepared, admission).map_err(refusal)
}
