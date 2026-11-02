//! Return the intact displayed plan after applying one authorized action.
pub(in crate::app) struct PreviewApplyResult {
    pub preview: crate::bisync::Preview,
    pub action: crate::bisync::Action,
    pub result: Result<String, String>,
}
