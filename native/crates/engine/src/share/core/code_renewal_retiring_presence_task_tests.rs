//! E3 (Restbeleg M34, E17 and E31): the presence of a retiring lookup id
//! carries no direct address. The check is not empty: the node publishes
//! direct addresses (`candidates()`), and the presence would carry them if
//! `build_retiring_presence` used them.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::build_retiring_presence;
use crate::share::backend::ShareIrohNode;
use crate::share::power::test_support::{auth_state, identity, NO_RELAY};

const RELATION: &str = "retiring-lookup-e3";
const SECRET: [u8; 32] = [9; 32];
/// How long the node may take to publish its direct addresses.
const SETTLE: Duration = Duration::from_secs(20);

/// A running node that stops sharing when the test ends.
struct Stop(Arc<ShareIrohNode>);

impl Drop for Stop {
    fn drop(&mut self) {
        let _ = self.0.stop_sharing();
    }
}

/// The direct addresses of `node`, once its endpoint has published them.
fn direct_addresses(node: &ShareIrohNode) -> Vec<String> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let addresses = node.candidates();
        if !addresses.is_empty() || Instant::now() >= deadline {
            return addresses;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn code_renewal_task_retiring_presence_carries_no_direct_address() {
    let host = identity("retiring host").expect("identity");
    let (events, _received) = crossbeam_channel::bounded(256);
    let node = ShareIrohNode::start(
        NO_RELAY,
        &host,
        Arc::new(Mutex::new(auth_state(&host))),
        events,
    )
    .expect("start the node");
    let _stop = Stop(node.clone());
    let direct = direct_addresses(&node);
    assert!(
        !direct.is_empty(),
        "the node publishes no direct address; the check would be empty"
    );

    let presence =
        build_retiring_presence(RELATION, &host, &SECRET, &node).expect("retiring presence");
    assert!(
        presence.candidates.is_empty(),
        "candidates: {:?}",
        presence.candidates
    );
    let shown = format!("{presence:?}");
    for address in &direct {
        assert!(
            !shown.contains(address.as_str()),
            "direct address {address} in the retiring presence"
        );
    }
}
