//! Agent plugin repair of 2026-11-14, cause U1: a daemon whose image lies outside every
//! installation tells the runtime so (instead of `reconnect_required`), and a
//! named root that does not load is still refused.
use super::reply_for;
use crate::mcp_launcher::{
    RuntimeHello, DAEMON_OUTSIDE_INSTALLATION, IPC_CONTRACT, STATE_CONTRACT,
};

fn hello(root: Option<String>) -> RuntimeHello {
    RuntimeHello {
        installation_id: "installation".into(),
        runtime_sha256: "a".repeat(64),
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
        installation_root: root,
    }
}

#[test]
fn plugin_task_daemon_outside_an_installation_names_the_reason() {
    let answer = reply_for(Ok(None), &hello(None), "generation");
    assert!(!answer.accepted);
    assert_eq!(
        answer.error_code.as_deref(),
        Some(DAEMON_OUTSIDE_INSTALLATION)
    );
    assert_eq!(answer.daemon_generation, "generation");
    assert_eq!(answer.ipc_contract_id, IPC_CONTRACT);
    assert_eq!(answer.shared_state_contract_id, STATE_CONTRACT);
}

#[test]
fn plugin_task_daemon_refuses_a_root_that_is_no_installation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_string_lossy().into_owned();
    let answer = reply_for(Ok(None), &hello(Some(root)), "generation");
    assert!(!answer.accepted);
    assert_eq!(answer.error_code.as_deref(), Some("reconnect_required"));
    let unreadable = reply_for(Err("image".into()), &hello(None), "generation");
    assert_eq!(unreadable.error_code.as_deref(), Some("reconnect_required"));
}
