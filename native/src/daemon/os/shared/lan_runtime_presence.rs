//! Private announcement, bounded sighting admission and contact reconciliation.
use super::*;

impl LanRuntime {
    pub(super) fn refresh_announcement(&mut self, input: &LanTickInput<'_>) {
        let Some(presence) = &self.presence else {
            return;
        };
        let addresses = self.facts.iter().filter(|facts| facts.up && !facts.loopback)
            .flat_map(|facts| facts.addrs.iter().copied()).collect();
        let Some((announcement, proof)) = self.authenticator.announcement(
            input.ports, input.uplink_advisory, addresses, input.now) else {
            presence.withdraw();
            return;
        };
        if let Err(error) = presence.announce_authenticated(&announcement, &proof) {
            self.presence_error = Some(error);
        }
    }

    pub(super) fn drain_presence_events(&mut self) {
        let Some(presence) = &self.presence else {
            return;
        };
        let own = presence
            .announced()
            .map(|announcement| announcement.hashed_id);
        let drained: Vec<LanEvent> = presence.events().try_iter().take(64).collect();
        for event in drained {
            match event {
                LanEvent::Seen(sighting) => {
                    if own.as_deref() == Some(sighting.id.as_str()) {
                        continue;
                    }
                    if sighting.id.len() != 16 { continue; }
                    if self.sightings.contains_key(&sighting.id) || self.sightings.len() < 1024 {
                        self.sightings.insert(sighting.id.clone(), sighting);
                    }
                }
                LanEvent::SeenAuthenticated { sighting, proof } => {
                    if own.as_deref() == Some(sighting.id.as_str())
                        || self.authenticator.authenticate(&sighting, &proof, crate::share::core_now_secs()).is_none() {
                        continue;
                    }
                    if self.sightings.len() >= 1024 && !self.sightings.contains_key(&sighting.id) {
                        // A verified contact can reclaim an unverified hint.
                        let victim = self.sightings.keys().find(|id| !self.proofs.contains_key(*id)).cloned();
                        if let Some(victim) = victim { self.sightings.remove(&victim); }
                        else { continue; }
                    }
                    self.proofs.insert(sighting.id.clone(), proof);
                    self.sightings.insert(sighting.id.clone(), sighting);
                }
                LanEvent::Lost(id) => {
                    if self.proofs.contains_key(&id) { continue; }
                    self.sightings.remove(&id);
                    self.proofs.remove(&id);
                }
                LanEvent::Error(error) => self.presence_error = Some(error),
            }
        }
    }

    pub(super) fn expire_sightings(&mut self, now: i64) {
        self.sightings
            .retain(|_, sighting| sighting.seen_at.saturating_add(LAN_PRESENCE_TTL_SECS) >= now);
        self.proofs.retain(|id, proof| self.sightings.contains_key(id) && proof.expires_at >= now);
    }

    /// Compare the current sightings with what was reported last time and
    /// emit the difference as events.
    pub(super) fn reconcile(&mut self, contacts: &[DirectContact], now: i64) -> Vec<ShareEvent> {
        let links = self.classified_links();
        let scopes: Vec<u32> = links
            .iter()
            .filter(|(facts, class)| *class != LinkClass::Inactive && facts.index != 0)
            .map(|(facts, _)| facts.index)
            .collect();
        let mut seen_now: HashMap<String, (Vec<String>, bool)> = HashMap::new();
        let mut seen_hashes = Vec::new();
        let mut authenticated_contacts = Vec::new();
        let legacy_contacts: HashMap<_, _> = contacts.iter()
            .filter(|contact| contact.access_state == crate::share::DirectAccessState::Accepted
                && contact.remote_device_id.is_some())
            .filter_map(|contact| Some((lan_presence_match::contact_lan_id(contact)?, contact.id.clone())))
            .collect();
        let mut unknown = 0usize;
        for sighting in self.sightings.values() {
            let authenticated = self.proofs.get(&sighting.id)
                .and_then(|proof| self.authenticator.authenticate(sighting, proof, now));
            let matched = authenticated.as_ref().map(|id| (id.clone(),
                lan_presence_match::candidates_for(sighting, &scopes)))
                .or_else(|| (sighting.id.len() == 16).then(||
                    legacy_contacts.get(&sighting.id).map(|id| (id.clone(),
                        lan_presence_match::candidates_for(sighting, &scopes)))).flatten());
            match matched {
                Some((contact_id, candidates)) => {
                    let entry = seen_now.entry(contact_id.clone()).or_insert_with(|| (Vec::new(), false));
                    entry.0.extend(candidates);
                    entry.0.sort();
                    entry.0.dedup();
                    if authenticated.is_some() {
                        // The beacon only provides a route. Own uplink is
                        // learned from the fresh pinned status channel.
                        entry.1 = self.link_facts.iter().find(|fact| fact.fresh(now)
                            && matches!(&fact.pin.origin, lan_link_facts::PinOrigin::Contact { id, .. } if id == &contact_id))
                            .is_some_and(|fact| fact.peer_uplink);
                        seen_hashes.push(sighting.id.clone());
                        authenticated_contacts.push(contact_id);
                    }
                }
                None => unknown += 1,
            }
        }
        self.unknown_devices = unknown;
        self.reported_hashes = seen_hashes;
        self.authenticated_contacts = authenticated_contacts;
        let mut events = Vec::new();
        for (contact_id, (candidates, uplink)) in &seen_now {
            let unchanged =
                self.reported
                    .get(contact_id)
                    .is_some_and(|(previous, previous_uplink)| {
                        previous == candidates && previous_uplink == uplink
                    });
            let stale = contacts
                .iter()
                .find(|contact| &contact.id == contact_id)
                .is_some_and(|contact| {
                    contact
                        .lan_seen_at
                        .is_none_or(|seen| seen.saturating_add(LAN_PRESENCE_TTL_SECS / 2) < now)
                });
            if !unchanged || stale {
                events.push(ShareEvent::LanPeerSeen {
                    contact_id: contact_id.clone(),
                    candidates: candidates.clone(),
                    uplink: *uplink,
                });
            }
        }
        for contact_id in self.reported.keys() {
            if !seen_now.contains_key(contact_id) {
                events.push(ShareEvent::LanPeerLost {
                    contact_id: contact_id.clone(),
                });
            }
        }
        self.reported = seen_now;
        events
    }

}
