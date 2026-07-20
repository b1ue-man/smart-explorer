use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use iroh::endpoint::Connection;
use iroh::{EndpointAddr, EndpointId, RelayUrl, TransportAddr};

use super::core::{eio, now_secs, verify_hmac};
use super::fs::ShareExportConfig;
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::types::{DirectGrantState, PeerEndpoint, PeerPresence, ShareAuthState, ShareScope};
use super::wire::PeerHello;

#[derive(Clone, Debug)]
pub(super) struct IncomingSession {
    hello: PeerHello,
}

pub(super) fn authenticate_incoming_session(
    hello: &PeerHello,
    remote_node: &str,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> io::Result<IncomingSession> {
    if hello.node_id != remote_node {
        return Err(eio("Iroh NodeId passt nicht zum Session-Handshake"));
    }
    let session = IncomingSession {
        hello: hello.clone(),
    };
    session.authorize(auth)?;
    Ok(session)
}

impl IncomingSession {
    /// Re-evaluates the identity, grant, relation secret, and export policy for
    /// every accepted filesystem stream. The QUIC connection alone is never an
    /// authorization cache.
    pub(super) fn authorize(
        &self,
        auth: &Arc<Mutex<ShareAuthState>>,
    ) -> io::Result<ShareExportConfig> {
        let state = auth.lock().map_err(|_| eio("Share-Auth gesperrt"))?;
        self.authorize_state(&state)
    }

    fn authorize_state(&self, state: &ShareAuthState) -> io::Result<ShareExportConfig> {
        let hello = &self.hello;
        match hello.relation_kind.as_str() {
            "direct" if hello.relation_id == state.identity.direct_lookup_id => {
                if !state.direct_online {
                    return Err(eio("Direktverbindung ist offline"));
                }
                let grant = state
                    .direct_grants
                    .iter()
                    .find(|g| {
                        g.device_id == hello.device_id
                            && g.state == DirectGrantState::Accepted
                            && g.public_key == hello.public_key
                            && g.node_id == hello.node_id
                    })
                    .ok_or_else(|| eio("Direktfreigabe nicht akzeptiert"))?;
                if !fingerprint_matches(&grant.public_key, &grant.fingerprint) {
                    return Err(eio("Direktfreigabe hat ungueltigen Fingerprint"));
                }
                let payload = session_payload(
                    "direct",
                    &hello.relation_id,
                    &hello.device_id,
                    &state.identity.device_id,
                    &hello.node_id,
                    &state.identity.node_id,
                    &hello.session_nonce,
                );
                if !verify_hmac(&state.direct_secret, &payload, &hello.session_proof) {
                    return Err(eio("Session-Proof ungueltig"));
                }
                Ok(state.default_direct_exports.clone())
            }
            "room" => {
                let room = state
                    .rooms
                    .iter()
                    .find(|r| r.room_id == hello.relation_id && r.auto_join)
                    .ok_or_else(|| eio("Unbekannter Raum"))?;
                let member = room
                    .members
                    .iter()
                    .find(|m| m.device_id == hello.device_id && !m.blocked)
                    .ok_or_else(|| eio("Geraet nicht im Raum"))?;
                if member.node_id != hello.node_id || member.public_key != hello.public_key {
                    return Err(eio("Raumgeraet hat Identitaetskonflikt"));
                }
                if !fingerprint_matches(&member.public_key, &member.fingerprint) {
                    return Err(eio("Raumgeraet hat ungueltigen Fingerprint"));
                }
                let secret = ShareProfiles::room_secret_checked(room)
                    .map_err(eio)?
                    .ok_or_else(|| eio("Raum-Secret fehlt"))?;
                let payload = session_payload(
                    "room",
                    &hello.relation_id,
                    &hello.device_id,
                    &state.identity.device_id,
                    &hello.node_id,
                    &state.identity.node_id,
                    &hello.session_nonce,
                );
                if !verify_hmac(&secret, &payload, &hello.session_proof) {
                    return Err(eio("Session-Proof ungueltig"));
                }
                Ok(room.exports.clone())
            }
            _ => Err(eio("Unbekannte oder nicht autorisierte Relation")),
        }
    }
}

pub(super) fn endpoint_addr(
    presence: &PeerPresence,
    local_addr: &EndpointAddr,
) -> io::Result<EndpointAddr> {
    if !presence.is_current_at(now_secs()) {
        return Err(eio("Peer-Presence ist abgelaufen"));
    }
    let node: EndpointId = presence.node_id.parse().map_err(eio)?;
    let mut addrs: Vec<TransportAddr> = presence
        .candidates
        .iter()
        .filter_map(|candidate| candidate.parse::<SocketAddr>().ok())
        .map(TransportAddr::Ip)
        .collect();
    if let Some(relay) = parse_relay_url(&relay_url_from_endpoint(&presence.relay_url)) {
        addrs.push(TransportAddr::Relay(relay));
    }
    // Both peers rendezvous on the configured Share relay, but an older
    // presence may contain an address alias which is only reachable from the
    // publishing host (for example, loopback versus the public name). Add only
    // our current relay aliases for the remote EndpointId; local IP addresses
    // never describe a route to the peer.
    addrs.extend(local_addr.relay_urls().cloned().map(TransportAddr::Relay));
    Ok(EndpointAddr::from_parts(node, addrs))
}

pub(super) fn transport_label(connection: &Connection) -> &'static str {
    let paths = connection.paths();
    paths
        .iter()
        .find(|path| path.is_selected())
        .map(|path| if path.is_relay() { "relay" } else { "direct" })
        .unwrap_or("unknown")
}

pub(super) fn relation_kind_id(endpoint: &PeerEndpoint) -> (&'static str, String) {
    match &endpoint.scope {
        ShareScope::Direct { .. } => ("direct", endpoint.presence.relation_id.clone()),
        ShareScope::Room { room_id } => ("room", room_id.clone()),
    }
}

pub(super) fn session_key(endpoint: &PeerEndpoint) -> String {
    let (kind, relation_id) = relation_kind_id(endpoint);
    format!("{kind}:{relation_id}:{}", endpoint.presence.node_id)
}

pub(super) fn session_payload(
    kind: &str,
    relation_id: &str,
    from_device: &str,
    to_device: &str,
    from_node: &str,
    to_node: &str,
    nonce: &str,
) -> String {
    format!(
        "smart-explorer/share/session/v3|{kind}|{relation_id}|{from_device}|{to_device}|{from_node}|{to_node}|{nonce}"
    )
}

#[cfg(test)]
pub(super) fn relay_url_from_signal(config: &str) -> String {
    config
        .split([',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(relay_url_from_endpoint)
        .find(|value| parse_relay_url(value).is_some())
        .unwrap_or_default()
}

pub(super) fn relay_urls_from_signal(config: &str) -> Vec<RelayUrl> {
    let mut relay_urls = Vec::new();
    for endpoint in config
        .split([',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Some(relay_url) = parse_relay_url(&relay_url_from_endpoint(endpoint)) else {
            continue;
        };
        if !relay_urls.contains(&relay_url) {
            relay_urls.push(relay_url);
        }
    }
    relay_urls
}

fn parse_relay_url(value: &str) -> Option<RelayUrl> {
    let relay_url = value.parse::<RelayUrl>().ok()?;
    matches!(relay_url.scheme(), "http" | "https").then_some(relay_url)
}

fn relay_url_from_endpoint(endpoint: &str) -> String {
    let normalized = if let Some(rest) = endpoint.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = endpoint.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if endpoint.starts_with("https://") || endpoint.starts_with("http://") {
        endpoint.to_string()
    } else if let Some(rest) = endpoint.strip_prefix("tcp://") {
        format!("http://{}", relay_tcp_addr(rest))
    } else if endpoint.contains("://") {
        endpoint.to_string()
    } else {
        format!("http://{}", relay_tcp_addr(endpoint))
    };
    trim_url_path(&normalized)
}

fn relay_tcp_addr(addr: &str) -> String {
    let addr = normalize_tcp_addr(addr);
    if let Ok(mut socket) = addr.parse::<SocketAddr>() {
        socket.set_port(socket.port().saturating_add(1));
        return socket.to_string();
    }
    if let Some((host, port)) = split_host_port(&addr) {
        if let Ok(port) = port.parse::<u16>() {
            return format!("{host}:{}", port.saturating_add(1));
        }
    }
    addr
}

fn split_host_port(addr: &str) -> Option<(&str, &str)> {
    let (host, port) = addr.rsplit_once(':')?;
    if port.contains(']') {
        return None;
    }
    Some((host, port))
}

fn normalize_tcp_addr(addr: &str) -> String {
    let addr = addr.trim().trim_end_matches('/');
    if addr.is_empty() || addr.starts_with('[') || addr.rsplit_once(':').is_some() {
        addr.to_string()
    } else {
        format!("{addr}:51820")
    }
}

fn trim_url_path(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let authority = rest.split('/').next().unwrap_or(rest);
        format!("{scheme}://{authority}")
    } else {
        url.to_string()
    }
}
