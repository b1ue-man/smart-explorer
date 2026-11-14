//! E52 (plan Ergänzung 1, L9): the LAN announcement of 63 expiring generations
//! reaches a browser completely. The announcement is registered by the production
//! path (`announce_authenticated`) on a daemon that keeps the loopback interface
//! enabled. A second daemon, as another device would run it, browses the same mDNS
//! service type once the announcer's own announcements are over, so the instances
//! reach it as the answer to its query. Needs multicast on `lo`.
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::bounded;

use super::super::{LanAnnouncement, LanPresence, LanSighting, RetiringLanAnnouncement, SERVICE};
use crate::share::lan_privacy::{
    make_proof, rotating_id, seal_proof, LanProof, ID_EPOCH_SECS, PROOF_LIFETIME_SECS,
};

const GENERATIONS: u8 = 63;
const PORT: u16 = 51820;
const WAIT: Duration = Duration::from_secs(30);
/// Time the announcer has for probing and its unsolicited announcements before
/// the browser starts.
const ANNOUNCED: Duration = Duration::from_secs(5);

/// The browsing daemon; it shuts down when the test ends, also on a panic.
struct Browser(mdns_sd::ServiceDaemon);

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}

/// The sighting under the lookup id that `secret` yields for `node` now.
fn sighting_under(node: &str, secret: &[u8; 32], address: IpAddr, now: i64) -> LanSighting {
    LanSighting {
        id: rotating_id(node, secret, now.div_euclid(ID_EPOCH_SECS)).expect("lookup id"),
        addrs: vec![address],
        p4: PORT,
        p6: 0,
        uplink: false,
        seen_at: now,
    }
}

/// A proof of `sighting`, signed by `key` and sealed under `secret`.
fn sealed_proof(
    sighting: &LanSighting,
    key: &iroh::SecretKey,
    secret: &[u8; 32],
    now: i64,
) -> LanProof {
    let mut proof = make_proof(
        sighting,
        now.div_euclid(ID_EPOCH_SECS),
        now + PROOF_LIFETIME_SECS,
        key,
    );
    proof.sealed = seal_proof(sighting, &proof, secret);
    proof
}

#[test]
#[ignore = "Multicast auf lo; Suite crates-task"]
fn code_renewal_grenze_task_sixty_three_generations_are_all_announced() {
    let key = iroh::SecretKey::from_bytes(&[7; 32]);
    let node = key.public().to_string();
    let now = crate::share::core_now_secs();
    let address: IpAddr = "192.168.1.5".parse().expect("address");

    // One retiring lookup id per generation, each sealed under its own secret.
    let retiring: Vec<RetiringLanAnnouncement> = (0..GENERATIONS)
        .map(|generation| {
            let secret = [generation + 1; 32];
            let sighting = sighting_under(&node, &secret, address, now);
            RetiringLanAnnouncement {
                hashed_id: sighting.id.clone(),
                proof: sealed_proof(&sighting, &key, &secret, now),
            }
        })
        .collect();
    // The current secret differs from every generation's secret (1..=63): an
    // equal secret yields the same lookup id and so the same instance names.
    let direct_secret = [GENERATIONS + 1; 32];
    let current = sighting_under(&node, &direct_secret, address, now);
    let current_proof = sealed_proof(&current, &key, &direct_secret, now);
    let announcement = LanAnnouncement {
        hashed_id: current.id.clone(),
        p4: PORT,
        p6: 0,
        uplink: false,
        plain_v2: true,
        retiring,
    };

    // The daemon is built directly, not through `LanPresence::start`, because
    // `start` turns the loopback interfaces off.
    let (_events_tx, events) = bounded(1);
    let presence = LanPresence {
        events,
        daemon: mdns_sd::ServiceDaemon::new().expect("mDNS daemon starts"),
        announced: Mutex::new(None),
    };
    presence
        .announce_authenticated(&announcement, &current_proof)
        .expect("the announcement registers");
    let expected = presence
        .announced
        .lock()
        .expect("announcement state is readable")
        .as_ref()
        .map(|(fullnames, _, _)| fullnames.iter().cloned().collect::<HashSet<String>>())
        .expect("the announcement is registered");
    // Two records per generation (sealed `v=3` and plain `v=2`), two for the current one.
    assert_eq!(expected.len(), 2 * usize::from(GENERATIONS) + 2);

    // The browser is a daemon of its own with an empty cache, as another device
    // runs it, started after the announcer's unsolicited announcements. The
    // instances then reach it as answers to its queries (L9), not as the
    // announcements made at registration.
    std::thread::sleep(ANNOUNCED);
    let browser = Browser(mdns_sd::ServiceDaemon::new().expect("mDNS browser starts"));
    let browse = browser.0.browse(SERVICE).expect("the mDNS browse starts");
    let (found_tx, found_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        while let Ok(event) = browse.recv() {
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                if found_tx.send(info.get_fullname().to_string()).is_err() {
                    break;
                }
            }
        }
    });

    let deadline = Instant::now() + WAIT;
    let mut resolved: HashSet<String> = HashSet::new();
    while !expected.is_subset(&resolved) {
        let left = deadline.saturating_duration_since(Instant::now());
        match found_rx.recv_timeout(left) {
            Ok(fullname) => {
                resolved.insert(fullname);
            }
            Err(_) => break,
        }
    }
    let missing = expected.difference(&resolved).count();
    assert_eq!(
        missing,
        0,
        "{missing} of {} announced instances were not resolved within {} s",
        expected.len(),
        WAIT.as_secs()
    );
}
