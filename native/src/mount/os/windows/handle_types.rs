use crate::mount::HandleId;

#[derive(Clone, Copy)]
pub(super) enum NodeHandle {
    File(HandleId),
    Directory,
}

#[derive(Clone)]
pub(super) struct HandleSnapshot {
    pub(super) node: NodeHandle,
    pub(super) path: String,
    pub(super) delete_requested: bool,
    pub(super) delete_committed: bool,
}
