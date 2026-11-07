//! r-entdeckung-5 (S52, suite-2-entdeckung): only a verified direct request
//! or decision keeps a device awake in low-power operation. A forged line
//! costs no wake time.
use std::sync::atomic::{AtomicU32, AtomicUsize};

use super::*;
use crate::share::configuration_runtime::RuntimeConfiguration;
use crate::share::core::{hmac_proof, now_secs, presence_payload, public_fingerprint};
use crate::share::direct_protocol::{DirectPeerIdentity, DirectRequestId, SignedDirectRequest};
use crate::share::identity::ShareIdentity;
use crate::share::power::SIGNAL_ACTIVITY_HOLD_MS;
use crate::share::signal_auth::{handle_server_msg, ServerMsgOutcome};
use crate::share::tracked_signal_dispatch::{
    dispatch_server_line, handle_tracked_server_message, SignalDispatchOutcome,
};
use crate::share::types::PeerPresence;
use crate::share::wire::TrackedDirectServerMsg;

static HOLDS: AtomicUsize = AtomicUsize::new(0);
static LAST_HOLD_MS: AtomicU32 = AtomicU32::new(0);

fn count_hold(hold_ms: u32) {
    HOLDS.fetch_add(1, Ordering::SeqCst);
    LAST_HOLD_MS.store(hold_ms, Ordering::SeqCst);
}

/// An unsigned (older device) request presence of a stranger holding the
/// Direct code; `secret` makes the relation proof valid or forged.
fn stranger_request(lookup_id: &str, secret: &[u8], nonce: &str) -> PeerPresence {
    let key = iroh::SecretKey::from_bytes(&[44; 32]);
    let public = key.public().to_string();
    let mut presence = PeerPresence {
        kind: "direct".into(),
        relation_id: lookup_id.into(),
        device_id: "stranger-device".into(),
        device_name: "Stranger".into(),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        relay_url: String::new(),
        candidates: Vec::new(),
        expires_at: now_secs() + 300,
        nonce: nonce.into(),
        proof: String::new(),
    };
    presence.proof = hmac_proof(
        secret,
        &presence_payload(
            "direct",
            &presence.relation_id,
            &presence.device_id,
            &presence.public_key,
            &presence.node_id,
            &presence.relay_url,
            &presence.candidates,
            presence.expires_at,
            &presence.nonce,
        ),
    );
    presence
}

fn legacy_line(lookup_id: &str, presence: &PeerPresence) -> String {
    serde_json::json!({
        "t": "direct_access_request",
        "lookup_id": lookup_id,
        "presence": presence,
    })
    .to_string()
}

/// A tracked request for `local`, sealed with `secret`.
fn tracked_request(local: &ShareIdentity, secret: &[u8], nonce: &str) -> TrackedDirectServerMsg {
    let key = iroh::SecretKey::from_bytes(&[45; 32]);
    let now = now_secs();
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        local.direct_lookup_id.clone(),
        DirectPeerIdentity::from_secret("tracked-requester", "Requester", &key),
        DirectPeerIdentity::pinned_target(local.node_id.clone(), local.fingerprint.clone()),
        now,
        now + 300,
        nonce,
        None,
        secret,
        &key,
    )
    .expect("signed request");
    TrackedDirectServerMsg::Request { request }
}

#[test]
fn entdeckung_task_signal_lines_report_verified_activity_only_for_verified_requests() {
    let local = identity("hold outcome local").expect("identity");
    let state = auth_state(&local);
    let secret = state.direct_secret.clone();
    let auth = Arc::new(Mutex::new(state));
    let (events, _received) = unbounded::<ShareEvent>();
    let lookup = local.direct_lookup_id.clone();

    let forged = stranger_request(&lookup, &[9u8; 32], "forged");
    assert_eq!(
        handle_server_msg(&legacy_line(&lookup, &forged), &auth, &events, false),
        ServerMsgOutcome::default(),
        "a forged request is no activity"
    );
    assert_eq!(
        handle_server_msg(r#"{"t":"pong"}"#, &auth, &events, false),
        ServerMsgOutcome {
            pong: true,
            verified_activity: false
        }
    );
    let valid = stranger_request(&lookup, &secret, "valid");
    assert_eq!(
        handle_server_msg(&legacy_line(&lookup, &valid), &auth, &events, false),
        ServerMsgOutcome {
            pong: false,
            verified_activity: true
        }
    );

    // The tracked messages: forged -> false, verified -> true.
    assert!(!handle_tracked_server_message(
        tracked_request(&local, &[9u8; 32], "tracked-forged"),
        &auth,
        &events
    ));
    assert!(handle_tracked_server_message(
        tracked_request(&local, &secret, "tracked-valid"),
        &auth,
        &events
    ));
}

struct Rig {
    harness: Harness,
    signal: SignalConnection,
    _server: FakeServer,
    discovery: DiscoverySignalRuntime,
    sent: HashSet<String>,
}

impl Rig {
    fn new() -> Self {
        let harness = Harness::manual();
        let (signal, server) = tcp_pair();
        let discovery = discovery_runtime(&harness.auth);
        Self {
            harness,
            signal,
            _server: server,
            discovery,
            sent: HashSet::new(),
        }
    }

    fn dispatch(&mut self, line: &str, tracked_direct: bool) -> SignalDispatchOutcome {
        let mut configuration = RuntimeConfiguration {
            auth: &self.harness.auth,
            iroh: &self.harness.node,
            direct_requests_sent: &mut self.sent,
        };
        dispatch_server_line(
            line,
            tracked_direct,
            true,
            &mut self.discovery,
            &mut self.signal,
            &self.harness.auth,
            &self.harness.events.0,
            &mut configuration,
        )
    }
}

#[test]
fn entdeckung_task_a_forged_line_costs_no_wake_time_and_a_verified_one_exactly_one_hold() {
    let mut rig = Rig::new();
    rig.harness.power.set_low_power(true);
    rig.harness.power.set_activity_hook(count_hold);
    let (local, secret) = {
        let state = rig.harness.auth.lock().expect("auth");
        (state.identity.clone(), state.direct_secret.clone())
    };
    let lookup = local.direct_lookup_id.clone();
    let holds = || HOLDS.load(Ordering::SeqCst);

    // Forged lines of every kind: no hold.
    let forged = stranger_request(&lookup, &[9u8; 32], "forged-legacy");
    rig.dispatch(&legacy_line(&lookup, &forged), false);
    assert_eq!(holds(), 0, "forged legacy request");
    let forged_tracked =
        serde_json::to_string(&tracked_request(&local, &[9u8; 32], "forged-tracked"))
            .expect("tracked json");
    rig.dispatch(&forged_tracked, true);
    assert_eq!(holds(), 0, "forged tracked request");
    let accepted = r#"{"t":"direct_access_accepted","lookup_id":"x","requester_device_id":"y","accepted":true,"presence":null,"msg":null}"#;
    rig.dispatch(accepted, true);
    assert_eq!(holds(), 0, "legacy decision while tracked_direct is agreed");

    // A verified legacy request: exactly one hold of the activity length.
    let valid = stranger_request(&lookup, &secret, "valid-legacy");
    rig.dispatch(&legacy_line(&lookup, &valid), false);
    assert_eq!(holds(), 1, "verified legacy request");
    assert_eq!(LAST_HOLD_MS.load(Ordering::SeqCst), SIGNAL_ACTIVITY_HOLD_MS);

    // A verified tracked request, after the first hold ran out.
    rig.harness.clock.advance(Duration::from_secs(20));
    let valid_tracked = serde_json::to_string(&tracked_request(&local, &secret, "valid-tracked"))
        .expect("tracked json");
    rig.dispatch(&valid_tracked, true);
    assert_eq!(holds(), 2, "verified tracked request");

    // A pong keeps the connection alive but holds nothing.
    rig.harness.clock.advance(Duration::from_secs(20));
    assert!(matches!(
        rig.dispatch(r#"{"t":"pong"}"#, false),
        SignalDispatchOutcome::Pong
    ));
    assert_eq!(holds(), 2, "pong");
}
