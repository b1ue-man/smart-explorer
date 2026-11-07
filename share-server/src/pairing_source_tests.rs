//! Pairing by source (R-SERVER-07): keys cost nothing, so one source holds at
//! most one pairing per offer and its share of the start window.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh_base::SecretKey;

use crate::discovery_state::{prepare_exchange_locked, DiscoveryOffer};
use crate::limits::SourceKey;
use crate::protocol::{DiscoveryAdvertisement, DiscoveryKind};
use crate::state::{register_client_with_identity, ClientIdentity, State};
use crate::{discovery, Writer};

const OFFER: &str = "offer";
const ALREADY_PAIRING: &str = "connector already pairing with this offer";
const RATE_EXCEEDED: &str = "discovery offer pairing attempt rate exceeded";

/// Registers a client with `seed`'s key from `source`; the key is proven for
/// a key login client and only named for an older one.
fn register(state: &Arc<Mutex<State>>, source: SourceKey, seed: u8, proven: bool) -> u64 {
    let key = SecretKey::from_bytes(&[seed; 32]);
    let identity = if proven {
        ClientIdentity::Proven(key.public())
    } else {
        ClientIdentity::LegacyClaimed(key.public())
    };
    let (writer, _receiver) = Writer::test_channel(32);
    register_client_with_identity(
        state,
        writer,
        source,
        format!("device-{seed}"),
        HashSet::from([discovery::CAPABILITY.into()]),
        identity,
    )
    .expect("registration")
}

fn source(last: u8) -> SourceKey {
    SourceKey::Ipv4([192, 0, 2, last])
}

#[test]
fn one_source_pairs_once_per_offer_and_starts_at_most_twelve_a_minute() {
    let state = Arc::new(Mutex::new(State::default()));
    let publisher = register(&state, source(1), 80, true);
    let first = register(&state, source(90), 81, true);
    let second = register(&state, source(90), 82, true);
    let other_source = register(&state, source(91), 83, true);
    let proxied = [
        register(&state, source(100), 84, false),
        register(&state, source(101), 85, false),
    ];
    let now = Instant::now();
    let mut state = state.lock().expect("state");
    state.discovery_offers.insert(
        OFFER.into(),
        DiscoveryOffer::new(
            publisher,
            DiscoveryAdvertisement {
                discovery_id: OFFER.into(),
                offer_id: "local-offer".into(),
                kind: DiscoveryKind::Direct,
                display_alias: "Alias".into(),
                suite: "opaque".into(),
                version: 1,
                expires_at: i64::MAX,
            },
            now + Duration::from_secs(300),
        ),
    );
    let start = |state: &mut State, client: u64, exchange: &str| {
        let result = prepare_exchange_locked(state, client, OFFER, exchange, 3, now);
        state.discovery_exchanges.remove(exchange);
        result.map(|_| ())
    };

    // Two keys on one source: the second pairing with the same offer waits.
    assert!(prepare_exchange_locked(&mut state, first, OFFER, "first", 3, now).is_ok());
    assert_eq!(
        prepare_exchange_locked(&mut state, second, OFFER, "second", 3, now).err(),
        Some(ALREADY_PAIRING)
    );
    // Another source pairs at the same time.
    assert!(prepare_exchange_locked(&mut state, other_source, OFFER, "other", 3, now).is_ok());
    state.discovery_exchanges.clear();

    // The window counts the starts of the source, whichever key made them:
    // one start so far (`first`), eleven more reach twelve.
    for index in 1..12 {
        let client = if index % 2 == 0 { first } else { second };
        assert_eq!(start(&mut state, client, &format!("start-{index}")), Ok(()));
    }
    assert_eq!(start(&mut state, first, "start-13"), Err(RATE_EXCEEDED));
    assert_eq!(start(&mut state, second, "start-14"), Err(RATE_EXCEEDED));
    // A different source has its own window.
    assert_eq!(start(&mut state, other_source, "other-start"), Ok(()));

    // Behind a trusted proxy every client has its own principal and no
    // source: both pair with the offer at the same time.
    for id in proxied {
        state.clients.get_mut(&id).expect("client").source = SourceKey::ExternallyLimitedProxy;
    }
    for (id, exchange) in proxied.into_iter().zip(["proxy-first", "proxy-second"]) {
        assert!(prepare_exchange_locked(&mut state, id, OFFER, exchange, 3, now).is_ok());
    }
}
