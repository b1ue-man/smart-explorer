//! Task suite of the candidate path (E26): the production authenticator takes a
//! sealed `v=3` sighting with every signed address. A sighting with an injected
//! address keeps only the signed addresses, so the candidate selection never
//! receives the injected one.
use std::net::IpAddr;

use super::task_tests::{announcement_of, authenticator_for, KnownContact};
use crate::share::power::test_support::identity;

const NOW: i64 = 1_800;

#[test]
fn lan_kuerzung_task_the_candidate_path_takes_a_v3_sighting_only_with_every_address_signed() {
    let own = identity("lan kuerzung wahl").expect("identity");
    let (contact, key) = KnownContact::with_key("contact-kw", 41);
    let authenticator = authenticator_for(&own, std::slice::from_ref(&contact), NOW);
    let signed: IpAddr = "192.168.1.5".parse().expect("address");
    let injected: IpAddr = "192.168.5.9".parse().expect("address");

    // The sealed record names only the signed address: the contact is taken.
    let (sighting, sealed) = announcement_of(&contact, &key, &[signed], NOW, true);
    let accepted = authenticator.authenticate(&sighting, &sealed, NOW);
    assert_eq!(accepted, Some("contact-kw".to_string()));

    // The same seal with an address the signed list does not name: the contact is
    // still taken, and the sighting keeps only the signed address (E26), so the
    // injected address never reaches the dial candidates.
    let mut injected_sighting = sighting.clone();
    injected_sighting.addrs.push(injected);
    let cut = authenticator.authenticate_cut(&injected_sighting, &sealed, NOW);
    assert_eq!(cut.as_ref().map(|(id, _)| id.as_str()), Some("contact-kw"));
    assert_eq!(cut.map(|(_, kept)| kept.addrs), Some(vec![signed]));
}
