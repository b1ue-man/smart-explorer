//! Short private leases from live paired TLS links to the privileged worker.
use super::lan_link_facts::{AuthenticatedLanFact, MAX_LINK_PEERS};
use super::{LanSettings, ShareProfiles};

const FILE: &str = "lan_uplink_evidence.json";
const MAX_BYTES: u64 = 256 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    version: u8,
    published_at: i64,
    facts: Vec<AuthenticatedLanFact>,
}

pub(crate) fn publish(facts: &[AuthenticatedLanFact]) -> Result<(), String> {
    if facts.len() > MAX_LINK_PEERS { return Err("Zu viele LAN-Link-Nachweise".into()); }
    let now = super::core_now_secs();
    let evidence = Evidence { version: 1, published_at: now,
        facts: facts.iter().filter(|fact| fact.fresh(now)).cloned().collect() };
    let bytes = serde_json::to_vec(&evidence).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES { return Err("LAN-Nachweis ist zu gross".into()); }
    crate::support_dirs::write_private_atomic(&crate::support_dirs::app_data_file(FILE), &bytes)
        .map_err(|error| format!("LAN-Nachweis speichern: {error}"))
}

/// A worker never starts from mDNS. Recheck the full current profile pins,
/// opt-in and exact current OS interface before consuming a short TLS lease.
pub(crate) fn authorize(private_index: u32, facts: &[crate::net::InterfaceFacts]) -> Result<(), String> {
    let settings = LanSettings::load()?;
    if !settings.presence_enabled || !settings.uplink_sharing_enabled || !settings.uplink_setup_done {
        return Err("LAN-Uplink-Freigabe ist ausgeschaltet oder nicht eingerichtet".into());
    }
    let text = crate::support_dirs::read_private_text(&crate::support_dirs::app_data_file(FILE), MAX_BYTES)
        .map_err(|error| format!("LAN-Nachweis lesen: {error}"))?;
    let evidence: Evidence = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let now = super::core_now_secs();
    if now <= 0 || evidence.published_at <= 0 || evidence.version != 1
        || evidence.facts.len() > MAX_LINK_PEERS || evidence.published_at > now
        || now.saturating_sub(evidence.published_at) >= super::lan_link_facts::MAX_FACT_LIFETIME_SECS {
        return Err("LAN-Link-Nachweis ist veraltet oder ungueltig".into());
    }
    let home = crate::support_dirs::home_dir().map(|path| path.to_string_lossy().replace('\\', "/"));
    let profiles = ShareProfiles::load_checked(home)?;
    for fact in &evidence.facts {
        if !fact.peer_uplink && fact.interface.index == private_index
            && fact.current(now, &profiles.direct_contacts, &profiles.direct_grants, facts)
            && fact.can_share_on(facts, &[]) {
            return Ok(());
        }
    }
    Err("Kein frischer gepinnter TLS-LAN-Link auf diesem privaten Interface".into())
}
