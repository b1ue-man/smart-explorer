//! Signed peer facts crossing from the daemon to the privileged uplink worker.
use super::{lan_presence_match, lan_privacy, DirectAccessState, LanProof, LanSighting, ShareProfiles};

const FILE: &str = "lan_uplink_evidence.json";
const MAX_BYTES: u64 = 256 * 1024;
const MAX_PEERS: usize = 128;

#[derive(serde::Serialize, serde::Deserialize)]
struct Evidence {
    sighting: LanSighting,
    proof: LanProof,
}

pub(crate) fn publish<'a>(peers: impl Iterator<Item = (&'a LanSighting, &'a LanProof)>) -> Result<(), String> {
    let peers: Vec<_> = peers.take(MAX_PEERS).map(|(sighting, proof)| Evidence {
        sighting: sighting.clone(), proof: proof.clone(),
    }).collect();
    let bytes = serde_json::to_vec(&peers).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES { return Err("LAN-Nachweis ist zu gross".into()); }
    crate::support_dirs::write_private_atomic(&crate::support_dirs::app_data_file(FILE), &bytes)
        .map_err(|error| format!("LAN-Nachweis speichern: {error}"))
}

/// Verify private dial hints, but fail closed until the paired-link Iroh
/// channel proves both no uplink and the selected local private interface.
pub(crate) fn authorize(private_index: u32, facts: &[crate::net::InterfaceFacts]) -> Result<(), String> {
    let text = crate::support_dirs::read_private_text(&crate::support_dirs::app_data_file(FILE), MAX_BYTES)
        .map_err(|error| format!("LAN-Nachweis lesen: {error}"))?;
    let evidence: Vec<Evidence> = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    if evidence.len() > MAX_PEERS { return Err("Zu viele LAN-Nachweise".into()); }
    let home = crate::support_dirs::home_dir().map(|path| path.to_string_lossy().replace('\\', "/"));
    let profiles = ShareProfiles::load_checked(home)?;
    let links = crate::net::classify_links(facts, &[], None, &[]);
    let now = super::core_now_secs();
    for peer in evidence {
        if peer.sighting.uplink
            || !lan_presence_match::peer_interfaces(&peer.sighting, &links).contains(&private_index) {
            continue;
        }
        for contact in &profiles.direct_contacts {
            if contact.access_state != DirectAccessState::Accepted || contact.remote_device_id.is_none() { continue; }
            let node = if !contact.expected_node_id.is_empty() { Some(contact.expected_node_id.as_str()) }
                else { contact.remote_public_key.as_deref().or(contact.accepted_public_key.as_deref()) };
            let Some(node) = node else { continue; };
            let Some(mut secret) = ShareProfiles::direct_secret_checked(contact)? else { continue; };
            let valid = lan_privacy::verify_sighting(&peer.sighting, &peer.proof, node, &secret, now);
            secret.fill(0);
            if valid {
                return Err("Iroh-Linkbestaetigung fehlt; signierte LAN-Sichtung allein erlaubt keine Uplink-Freigabe".into());
            }
        }
    }
    Err("Kein frischer signierter Nachweis eines gekoppelten Geraets auf diesem Link".into())
}
