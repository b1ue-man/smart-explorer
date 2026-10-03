//! Bounded outbound probes using current Direct pins and private dial hints.
use super::*;

impl LanLinkTransport {
    pub(super) fn probe(self: &Arc<Self>, node: Arc<ShareIrohNode>) -> io::Result<()> {
        self.host()?;
        let auth = node.auth.try_lock().map_err(|_| busy())?;
        let now = now_secs();
        let mut peers = Vec::new();
        for contact in &auth.direct_contacts {
            if peers.len() >= MAX_LINK_PEERS {
                break;
            }
            let Some(pin) = lan_link_facts::contact_pin(contact) else {
                continue;
            };
            let Some(presence) =
                super::super::lan_presence_match::effective_presence(contact, now)
            else {
                continue;
            };
            let candidates: Vec<_> = presence
                .candidates
                .iter()
                .take(16)
                .filter_map(|candidate| crate::net::parse_candidate(candidate))
                .filter(|address| lan_link_facts::private_ip(address.ip()) && address.port() != 0)
                .map(TransportAddr::Ip)
                .collect();
            if !candidates.is_empty()
                && !peers
                    .iter()
                    .any(|(known, _): &(LanPeerPin, Vec<TransportAddr>)| {
                        known.node_id == pin.node_id
                    })
            {
                peers.push((pin, candidates));
            }
        }
        drop(auth);
        let mut state = self.state.try_lock().map_err(|_| busy())?;
        state.probes.retain(|id, at| {
            at.elapsed() < SESSION_DEADLINE + PROBE_BACKOFF
                && peers.iter().any(|(pin, _)| &pin.node_id == id)
        });
        let count = peers.len();
        if count > 0 {
            let offset = state.cursor % count;
            peers.rotate_left(offset);
            state.cursor = (offset + 1) % count;
        }
        let mut launches = Vec::new();
        for (pin, candidates) in peers {
            if state
                .probes
                .get(&pin.node_id)
                .is_some_and(|at| at.elapsed() < SESSION_DEADLINE + PROBE_BACKOFF)
                || state.channels.values().any(|channel| {
                    channel.connection.close_reason().is_none()
                        && channel.connection.remote_id().to_string() == pin.node_id
                })
            {
                continue;
            }
            let Ok(permit) = self.outbound.clone().try_acquire_owned() else {
                break;
            };
            let remote = pin.node_id.parse::<iroh::EndpointId>().map_err(eio)?;
            state.probes.insert(pin.node_id.clone(), Instant::now());
            launches.push((pin, EndpointAddr::from_parts(remote, candidates), permit));
        }
        drop(state);
        for (pin, addr, permit) in launches {
            let transport = self.clone();
            let task_node = node.clone();
            node.rt.spawn(async move {
                let _permit = permit;
                let connected = tokio::time::timeout(
                    ROUND_DEADLINE,
                    task_node.endpoint.connect(addr, LAN_LINK_ALPN),
                )
                .await;
                let Ok(Ok(connection)) = connected else {
                    return;
                };
                if connection.remote_id().to_string() != pin.node_id
                    || transport.register(&connection).is_err()
                {
                    close(&connection);
                    return;
                }
                let _ = super::super::lan_link_exchange::run(
                    task_node,
                    transport.clone(),
                    connection.clone(),
                    Some(pin),
                )
                .await;
                transport.remove(&connection);
            });
        }
        Ok(())
    }
}
