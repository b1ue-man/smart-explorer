//! E21 watch: a changed server or a restarted broker starts one sync.
use super::*;

#[test]
fn login_selbst_task_watch_follows_server_and_broker_restart() {
    // The configured server changes: the first look only records, the
    // change starts a sync, the same value again does not.
    let direct = GatewayNetwork {
        format: crate::login::core::model::VERSION,
        use_default_relays: false,
        relay_urls: Vec::new(),
        relay_pins: Vec::new(),
        allow_plaintext_relays: false,
    };
    let relayed = GatewayNetwork {
        relay_urls: vec!["https://relay.example.org/".into()],
        relay_pins: vec![[9u8; 32]],
        ..direct.clone()
    };
    let mut network = None;
    assert!(!note(&mut network, Some(direct.clone())));
    assert!(!note(&mut network, Some(direct)));
    assert!(note(&mut network, Some(relayed.clone())));
    assert!(!note(&mut network, Some(relayed)));
    // The broker restarts (a new process instance) or comes up after the
    // worker: its policy may still hold a grant revoked meanwhile.
    let mut broker = None;
    assert!(!note(&mut broker, None::<String>));
    assert!(note(&mut broker, Some("instance-1".to_string())));
    assert!(!note(&mut broker, Some("instance-1".to_string())));
    assert!(note(&mut broker, Some("instance-2".to_string())));
    // The watch never prompts: it only syncs a running service.
    use crate::login::core::self_plan::{decide, Installed, Plan};
    for installed in [Installed::Missing, Installed::Unavailable] {
        assert!(!matches!(
            decide(LoginTrigger::Watch, true, &installed, None, "0.5.197"),
            Plan::Install | Plan::Sync | Plan::SyncKeep(_)
        ));
    }
}
