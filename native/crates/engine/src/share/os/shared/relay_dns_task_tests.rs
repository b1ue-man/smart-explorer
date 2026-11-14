//! Milestone M6 (decisions E32, E33, E34; plan 2026-11-08).
//!
//! - E32: the vendor gate (`iroh::endpoint::RemoteKeyGate`, set on the Share
//!   endpoint by `share/core/blocked_keys.rs`) fails the TLS handshake of a
//!   removed device in both directions, also after an earlier session of the
//!   same key, so the device is never registered and gets none of this
//!   device's addresses (`ADD_ADDRESS`, probes for its `REACH_OUT`). A removed
//!   key that a relation admits again (accepted grant or contact, admitted
//!   member of an auto-join room) still connects, as in the accept loop.
//! - E33: Iroh's default resolver names no public fallback nameserver.
//! - E34: with an empty nameserver list (Android without a readable system
//!   configuration) the Share resolver answers A and AAAA through the system
//!   (`getaddrinfo`) without waiting for `DNS_TIMEOUT`.
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{presets, Connection, PortmapperConfig, RemoteKeyGate};
use iroh::{Endpoint, EndpointAddr, TransportAddr};

use crate::share::identity::code_renewal_registry as registry;
use crate::share::node::blocked_keys;
use crate::share::power::test_support::{auth_state, identity};
use crate::share::types::{DirectGrant, DirectGrantState, ShareAuthState};

const ALPN: &[u8] = b"smart-explorer/vendor-grenze-task/1";
/// Bound of one handshake attempt on loopback.
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("test runtime")
}

/// An endpoint on one IPv4 loopback socket, without relay and portmapper.
async fn bind(gate: Option<RemoteKeyGate>) -> Endpoint {
    let mut builder = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .expect("loopback address")
        .portmapper_config(PortmapperConfig::Disabled)
        .alpns(vec![ALPN.to_vec()]);
    if let Some(gate) = gate {
        builder = builder.remote_key_gate(gate);
    }
    builder.bind().await.expect("bind endpoint")
}

/// The direct loopback address of `endpoint`, as a device that still knows
/// its IP address dials it.
fn direct_addr(endpoint: &Endpoint) -> EndpointAddr {
    let socket = endpoint
        .bound_sockets()
        .into_iter()
        .next()
        .expect("bound loopback socket");
    EndpointAddr::from_parts(endpoint.id(), [TransportAddr::Ip(socket)])
}

/// One connection attempt of `client` to `server`: the connection each side
/// completed, if any.
async fn handshake(
    client: &Endpoint,
    server: &Endpoint,
) -> (Option<Connection>, Option<Connection>) {
    let dialing = async {
        tokio::time::timeout(HANDSHAKE_LIMIT, client.connect(direct_addr(server), ALPN))
            .await
            .ok()
            .and_then(Result::ok)
    };
    let accepting = async {
        let incoming = tokio::time::timeout(HANDSHAKE_LIMIT, server.accept())
            .await
            .ok()
            .flatten()?;
        tokio::time::timeout(HANDSHAKE_LIMIT, async move { incoming.await })
            .await
            .ok()
            .and_then(Result::ok)
    };
    tokio::join!(dialing, accepting)
}

/// The relations of a Share node without any grant, contact or room.
fn no_relations() -> Arc<Mutex<ShareAuthState>> {
    let own = identity("vendor-grenze-task").expect("identity");
    Arc::new(Mutex::new(auth_state(&own)))
}

/// The gate of a Share node with the relations `auth`.
fn gate(auth: &Arc<Mutex<ShareAuthState>>) -> Option<RemoteKeyGate> {
    Some(blocked_keys::remote_key_gate(auth.clone()))
}

/// An accepted Direct grant of the device `endpoint`.
fn accepted_grant(endpoint: &Endpoint) -> DirectGrant {
    let key = endpoint.id().to_string();
    DirectGrant {
        device_id: "vendor-grenze-task-device".to_string(),
        device_name: "vendor-grenze-task-device".to_string(),
        public_key: key.clone(),
        fingerprint: String::new(),
        node_id: key,
        state: DirectGrantState::Accepted,
        updated_at: 0,
        exec: Default::default(),
        write: false,
    }
}

/// Records `removed` as identities the user removed, under the local view
/// `local` (empty: none).
fn withdraw(local: &Endpoint, removed: &[&Endpoint]) {
    let removed = removed.iter().map(|ep| ep.id().to_string()).collect();
    registry::install(
        &local.id().to_string(),
        "",
        Vec::new(),
        removed,
        Default::default(),
    );
}

#[test]
fn vendor_grenze_task_removed_key_fails_the_handshake_in_both_directions() {
    runtime().block_on(async {
        // A removed device dials this host's direct address.
        let relations = no_relations();
        let host = bind(gate(&relations)).await;
        let removed = bind(None).await;
        withdraw(&host, &[&removed]);
        let refused = blocked_keys::refuses(&relations, &removed.id().to_string());
        let (dialed, accepted) = handshake(&removed, &host).await;
        withdraw(&host, &[]);
        assert!(refused, "the removed node id is not in the gate's set");
        assert!(dialed.is_none(), "the removed device completed a handshake");
        assert!(accepted.is_none(), "the host registered the removed device");

        // This device dials a removed host: no client flight with REACH_OUT.
        let device = bind(gate(&no_relations())).await;
        let removed_host = bind(None).await;
        withdraw(&device, &[&removed_host]);
        let (dialed, accepted) = handshake(&device, &removed_host).await;
        withdraw(&device, &[]);
        assert!(
            dialed.is_none(),
            "this device completed a handshake with a removed host"
        );
        assert!(accepted.is_none(), "the removed host completed a handshake");

        for endpoint in [host, removed, device, removed_host] {
            endpoint.close().await;
        }
    });
}

#[test]
fn vendor_grenze_task_removal_applies_to_the_next_handshake_without_resumption() {
    runtime().block_on(async {
        let host = bind(gate(&no_relations())).await;
        // The other device runs a build without the gate, so it stores every
        // session ticket a host sends and tries to resume with it.
        let device = bind(None).await;
        let (dialed, accepted) = handshake(&device, &host).await;
        assert!(
            dialed.is_some(),
            "an allowed device could not dial a gated host"
        );
        assert!(accepted.is_some(), "a gated host refused an allowed device");
        // Time for the NewSessionTicket an ungated host would send.
        tokio::time::sleep(Duration::from_millis(250)).await;
        drop((dialed, accepted));

        withdraw(&host, &[&device]);
        let (dialed, accepted) = handshake(&device, &host).await;
        withdraw(&host, &[]);
        assert!(
            dialed.is_none(),
            "the removed device completed a (resumed) handshake"
        );
        assert!(accepted.is_none(), "the host registered the removed device");

        host.close().await;
        device.close().await;
    });
}

#[test]
fn vendor_grenze_task_removed_key_admitted_again_keeps_connecting() {
    runtime().block_on(async {
        let relations = no_relations();
        let host = bind(gate(&relations)).await;
        let device = bind(None).await;
        let key = device.id().to_string();
        // Removed and not admitted again: refused.
        withdraw(&host, &[&device]);
        let refused_unknown = blocked_keys::refuses(&relations, &key);
        // Removed, but an accepted relation admits the key again (as a room
        // membership or a later contact would): the accept loop lets it in
        // (`!known && is_withdrawn`), so the gate does too.
        relations
            .lock()
            .expect("relations")
            .direct_grants
            .push(accepted_grant(&device));
        let refused_known = blocked_keys::refuses(&relations, &key);
        let (dialed, accepted) = handshake(&device, &host).await;
        // A state held elsewhere during the handshake refuses the removed key.
        let refused_busy = {
            let _held = relations.lock().expect("relations");
            blocked_keys::refuses(&relations, &key)
        };
        withdraw(&host, &[]);
        let refused_after_reset = {
            let _held = relations.lock().expect("relations");
            blocked_keys::refuses(&relations, &key)
        };
        assert!(refused_unknown, "a removed unknown key was not refused");
        assert!(!refused_known, "a removed key admitted again was refused");
        assert!(
            dialed.is_some(),
            "a removed key admitted again could not dial"
        );
        assert!(accepted.is_some(), "the host refused a key admitted again");
        assert!(refused_busy, "a busy state did not fail closed");
        assert!(
            !refused_after_reset,
            "a key that is not removed waited for the state"
        );
        host.close().await;
        device.close().await;
    });
}

/// E33 and E34 compare with the system configuration in `/etc/resolv.conf`
/// and `/etc/hosts` (Linux, the system Android is built on).
#[cfg(target_os = "linux")]
mod resolvers {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::time::Instant;

    use iroh::dns::{DnsResolver, FallbackMode, DNS_TIMEOUT};

    use super::super::public_fallback_tests::{addresses_in, system_nameservers, PUBLIC_RESOLVERS};
    use super::super::{ipv6, system_addresses, SystemFirstResolver};
    use super::{bind, runtime};

    fn public_resolvers() -> Vec<IpAddr> {
        PUBLIC_RESOLVERS
            .iter()
            .map(|address| address.parse().expect("literal"))
            .collect()
    }

    /// The Share resolver whose own resolver has no nameserver at all, as on
    /// Android when the system DNS configuration cannot be read and
    /// `FallbackMode::Never` adds none.
    fn share_resolver_without_nameservers() -> DnsResolver {
        let fallback = DnsResolver::builder()
            .with_fallback_mode(FallbackMode::Never)
            .build();
        let described = format!("{fallback:?}");
        let named = addresses_in(&described);
        assert!(named.is_empty(), "nameservers {named:?} in {described}");
        DnsResolver::custom(SystemFirstResolver { fallback })
    }

    #[test]
    fn dns_android_task_empty_nameservers_answer_a_through_the_system_at_once() {
        let (addrs, waited) = runtime().block_on(async {
            let resolver = share_resolver_without_nameservers();
            let started = Instant::now();
            let addrs =
                tokio::time::timeout(DNS_TIMEOUT, resolver.lookup_ipv4("localhost", DNS_TIMEOUT))
                    .await
                    .expect("the A lookup waited for DNS_TIMEOUT")
                    .expect("the A lookup failed")
                    .collect::<Vec<IpAddr>>();
            (addrs, started.elapsed())
        });
        assert!(
            addrs.contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)),
            "{addrs:?}"
        );
        assert!(waited < DNS_TIMEOUT, "waited {waited:?}");
    }

    #[test]
    fn dns_android_task_empty_nameservers_answer_aaaa_through_the_system_at_once() {
        let (system, addrs, waited) = runtime().block_on(async {
            let system = system_addresses("localhost", ipv6).await;
            let resolver = share_resolver_without_nameservers();
            let started = Instant::now();
            let addrs =
                tokio::time::timeout(DNS_TIMEOUT, resolver.lookup_ipv6("localhost", DNS_TIMEOUT))
                    .await
                    .expect("the AAAA lookup waited for DNS_TIMEOUT")
                    .expect("the AAAA lookup failed")
                    .collect::<Vec<IpAddr>>();
            (system, addrs, started.elapsed())
        });
        assert!(
            system.contains(&Ipv6Addr::LOCALHOST),
            "precondition: the system names ::1 for localhost, got {system:?}"
        );
        assert!(
            addrs.contains(&IpAddr::V6(Ipv6Addr::LOCALHOST)),
            "{addrs:?}"
        );
        assert!(waited < DNS_TIMEOUT, "waited {waited:?}");
    }

    #[test]
    fn vendor_grenze_task_default_resolver_has_no_public_fallback() {
        let public = public_resolvers();
        let (upstream, described) = runtime().block_on(async {
            let upstream = format!("{:?}", DnsResolver::new());
            let endpoint = bind(None).await;
            let described = format!("{:?}", endpoint.dns_resolver().expect("open endpoint"));
            endpoint.close().await;
            (upstream, described)
        });
        // Control: the Debug text of a resolver with Iroh's upstream fallback
        // mode names the public resolvers, so their absence below is a finding.
        assert!(
            addresses_in(&upstream)
                .iter()
                .any(|address| public.contains(address)),
            "control: no public resolver in {upstream}"
        );
        let system = system_nameservers();
        for address in addresses_in(&described) {
            assert!(
                !public.contains(&address) || system.contains(&address),
                "public resolver {address} in the default resolver {described}"
            );
        }
    }
}
