//! Daemon-token-authenticated typed device commands; no executable inputs.
use crate::share::{
    apps_wire::{AppsCommand, AppsResult},
    PeerOpenTarget,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AppsIpcRequest {
    Command {
        target: PeerOpenTarget,
        command: AppsCommand,
    },
    LocalGrantSnapshot {
        device_id: String,
        public_key: String,
    },
    SetLocalGrant {
        change: crate::apps::LocalAppGrantChange,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AppsIpcReply {
    Pending,
    Completed {
        node_id: String,
        device_id: String,
        result: AppsResult,
    },
    LocalGrantSnapshot {
        snapshot: crate::apps::LocalAppGrantSnapshot,
    },
    LocalGrantMutation {
        result: crate::daemon::ipc_host::exec_grant_journal::ExecGrantPersistResult,
    },
}
