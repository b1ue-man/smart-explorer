//! mDNS announcement and discovery of Smart Explorer Direct endpoints on the
//! local network (`_se-share._udp.local.`).
//!
//! The announcement carries a hashed endpoint id, the Iroh UDP ports and an
//! advisory uplink flag; matching and authentication happen elsewhere. The
//! daemon starts this once; failures are reported as explicit errors and the
//! owner retries later instead of assuming multicast works.
use std::collections::HashMap;
use std::sync::Mutex;

use crossbeam_channel::{bounded, Receiver, TrySendError};

use super::lan_presence_match::{LanSighting, LAN_PRESENCE_VERSION};
use super::lan_privacy::{LanProof, SealedLanProof, MAX_ADDRESSES, SEAL_NONCE_BYTES};

const SERVICE: &str = "_se-share._udp.local.";
const INSTANCE_PREFIX: &str = "se-";
/// Instance of the sealed `v=3` record. Older receivers find no hashed id in
/// this name and drop its 32-character id as an unknown `v=1` sighting.
const SEALED_INSTANCE_PREFIX: &str = "se3-";
/// TXT values stay under the DNS-SD 255-byte bound; the sealed advisory is
/// split into at most this many base64url chunks `c0..`.
const SEALED_CHUNK_CHARS: usize = 200;
const MAX_SEALED_CHUNKS: usize = 8;

/// Seconds between the mDNS daemon's checks of the network interfaces.
///
/// mdns-sd 0.11.5 checked every 30 s (`IP_CHECK_INTERVAL_MILLIS`); 0.21.6
/// defaults to `IP_CHECK_INTERVAL_IN_SECS_DEFAULT = 5` and so wakes the
/// daemon thread and reads all interfaces six times as often, around the
/// clock, also on a phone in the background. The owner (`lan_runtime.rs`)
/// gathers interface facts every few seconds itself and restarts or
/// re-announces on its own; the daemon check only has to pick up new
/// addresses for the auto-address announcement, which 30 s serves as before.
pub(crate) const IP_CHECK_INTERVAL_SECS: u32 = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanAnnouncement {
    pub hashed_id: String,
    pub p4: u16,
    pub p6: u16,
    pub uplink: bool,
    /// Also announce the plain `v=2` record next to the sealed `v=3` one:
    /// some audience contact has not yet shown that it reads `v=3`.
    pub plain_v2: bool,
    /// E17: the same announcement under each lookup id that runs out after
    /// a Direct-code renewal (own rotating id and seal from its secret), so
    /// holders that have not yet switched still find this device in the LAN.
    pub retiring: Vec<RetiringLanAnnouncement>,
}

#[path = "lan_presence_retiring.rs"]
mod retiring;
pub use retiring::RetiringLanAnnouncement;
use retiring::{same_proof, same_records};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanEvent {
    Seen(LanSighting),
    SeenAuthenticated {
        sighting: LanSighting,
        proof: LanProof,
    },
    Lost(String),
    Error(String),
}

/// Live mDNS browser/announcer. Dropping it stops both.
pub struct LanPresence {
    events: Receiver<LanEvent>,
    daemon: mdns_sd::ServiceDaemon,
    announced: Mutex<Option<(Vec<String>, LanAnnouncement, Option<LanProof>)>>,
}

impl LanPresence {
    pub fn start() -> Result<Self, String> {
        let daemon = mdns_sd::ServiceDaemon::new()
            .map_err(|error| format!("mDNS-Dienst konnte nicht gestartet werden: {error}"))?;
        configure_daemon(&daemon)?;
        let browse = daemon
            .browse(SERVICE)
            .map_err(|error| format!("mDNS-Suche konnte nicht gestartet werden: {error}"))?;
        let (tx, events) = bounded(256);
        std::thread::Builder::new()
            .name("se-lan-presence".into())
            .spawn(move || {
                while let Ok(event) = browse.recv() {
                    let mapped = match event {
                        mdns_sd::ServiceEvent::ServiceResolved(info) => event_from(&info),
                        mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                            id_from_fullname(&fullname).map(LanEvent::Lost)
                        }
                        _ => None,
                    };
                    if let Some(mapped) = mapped {
                        match tx.try_send(mapped) {
                            Ok(()) | Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                }
            })
            .map_err(|error| format!("mDNS-Thread konnte nicht gestartet werden: {error}"))?;
        Ok(Self {
            events,
            daemon,
            announced: Mutex::new(None),
        })
    }

    pub fn events(&self) -> &Receiver<LanEvent> {
        &self.events
    }

    pub fn announced(&self) -> Option<LanAnnouncement> {
        self.announced.lock().ok().and_then(|guard| {
            guard
                .as_ref()
                .map(|(_, announcement, _)| announcement.clone())
        })
    }

    /// Register (or replace) our own announcement. Unchanged announcements
    /// are a no-op.
    pub fn announce(&self, announcement: &LanAnnouncement) -> Result<(), String> {
        self.announce_with_proof(announcement, None)
    }

    pub fn announce_authenticated(
        &self,
        announcement: &LanAnnouncement,
        proof: &LanProof,
    ) -> Result<(), String> {
        self.announce_with_proof(announcement, Some(proof))
    }

    fn announce_with_proof(
        &self,
        announcement: &LanAnnouncement,
        proof: Option<&LanProof>,
    ) -> Result<(), String> {
        let mut guard = self
            .announced
            .lock()
            .map_err(|_| "mDNS-Ankuendigung ist gesperrt".to_string())?;
        if guard.as_ref().is_some_and(|(_, current, previous)| {
            same_records(current, announcement)
                && match (previous, proof) {
                    (Some(previous), Some(proof)) => same_proof(previous, proof),
                    (None, None) => true,
                    _ => false,
                }
        }) {
            return Ok(());
        }
        if let Some((fullnames, _, _)) = guard.take() {
            for fullname in fullnames {
                let _ = self.daemon.unregister(&fullname);
            }
        }
        let mut records = Vec::new();
        match proof {
            None => records.push((INSTANCE_PREFIX, plain_properties(announcement, None))),
            Some(proof) => {
                if let Some(sealed) = &proof.sealed {
                    records.push((
                        SEALED_INSTANCE_PREFIX,
                        sealed_properties(announcement, proof, sealed),
                    ));
                }
                if announcement.plain_v2 || proof.sealed.is_none() {
                    records.push((INSTANCE_PREFIX, plain_properties(announcement, Some(proof))));
                }
            }
        }
        let mut views = vec![announcement.clone(); records.len()];
        if proof.is_some() {
            retiring::append_records(announcement, &mut records, &mut views);
        }
        let mut fullnames = Vec::new();
        for ((prefix, properties), view) in records.into_iter().zip(&views) {
            match self.register(prefix, view, properties) {
                Ok(fullname) => fullnames.push(fullname),
                Err(error) => {
                    // Never leave a record behind that no later call removes.
                    for fullname in fullnames {
                        let _ = self.daemon.unregister(&fullname);
                    }
                    return Err(error);
                }
            }
        }
        *guard = Some((fullnames, announcement.clone(), proof.cloned()));
        Ok(())
    }

    fn register(
        &self,
        prefix: &str,
        announcement: &LanAnnouncement,
        properties: HashMap<String, String>,
    ) -> Result<String, String> {
        let instance = format!("{prefix}{}", announcement.hashed_id);
        let host = format!("{instance}.local.");
        let port = if announcement.p4 != 0 {
            announcement.p4
        } else {
            announcement.p6
        };
        let info = mdns_sd::ServiceInfo::new(SERVICE, &instance, &host, "", port, properties)
            .map_err(|error| format!("mDNS-Ankuendigung ungueltig: {error}"))?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        self.daemon
            .register(info)
            .map_err(|error| format!("mDNS-Ankuendigung fehlgeschlagen: {error}"))?;
        Ok(fullname)
    }

    pub fn withdraw(&self) {
        if let Ok(mut guard) = self.announced.lock() {
            if let Some((fullnames, _, _)) = guard.take() {
                for fullname in fullnames {
                    let _ = self.daemon.unregister(&fullname);
                }
            }
        }
    }
}

impl Drop for LanPresence {
    fn drop(&mut self) {
        self.withdraw();
        let _ = self.daemon.shutdown();
    }
}

/// Options every daemon of this module runs with.
fn configure_daemon(daemon: &mdns_sd::ServiceDaemon) -> Result<(), String> {
    let setup_error =
        |error: mdns_sd::Error| format!("mDNS-Dienst konnte nicht eingerichtet werden: {error}");
    daemon
        .set_ip_check_interval(IP_CHECK_INTERVAL_SECS)
        .map_err(setup_error)?;
    // mdns-sd >= 0.17 also uses loopback interfaces by default; 0.11 did
    // not. Keep announcing and browsing on the LAN interfaces only.
    daemon
        .disable_interface(vec![
            mdns_sd::IfKind::LoopbackV4,
            mdns_sd::IfKind::LoopbackV6,
        ])
        .map_err(setup_error)
}

fn clear_properties(announcement: &LanAnnouncement, version: &str) -> HashMap<String, String> {
    HashMap::from([
        ("v".to_string(), version.to_string()),
        ("id".to_string(), announcement.hashed_id.clone()),
        ("p4".to_string(), announcement.p4.to_string()),
        ("p6".to_string(), announcement.p6.to_string()),
        ("up".to_string(), u8::from(announcement.uplink).to_string()),
    ])
}

/// `v=1` without proof, `v=2` with the signature and the addresses in clear.
fn plain_properties(
    announcement: &LanAnnouncement,
    proof: Option<&LanProof>,
) -> HashMap<String, String> {
    let version = if proof.is_some() {
        "2"
    } else {
        LAN_PRESENCE_VERSION
    };
    let mut properties = clear_properties(announcement, version);
    if let Some(proof) = proof {
        properties.insert("e".into(), proof.epoch.to_string());
        properties.insert("x".into(), proof.expires_at.to_string());
        properties.insert("s".into(), proof.signature.clone());
        // Each TXT value remains under the DNS-SD 255-byte bound even
        // when a host has many IPv6 addresses.
        for (index, address) in proof.addresses.iter().take(MAX_ADDRESSES).enumerate() {
            properties.insert(format!("a{index}"), address.to_string());
        }
    }
    properties
}

/// `v=3`: epoch and expiry in clear, addresses and signature only sealed.
fn sealed_properties(
    announcement: &LanAnnouncement,
    proof: &LanProof,
    sealed: &SealedLanProof,
) -> HashMap<String, String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let mut properties = clear_properties(announcement, "3");
    properties.insert("e".into(), proof.epoch.to_string());
    properties.insert("x".into(), proof.expires_at.to_string());
    properties.insert("n".into(), URL_SAFE_NO_PAD.encode(sealed.nonce));
    let encoded = URL_SAFE_NO_PAD.encode(&sealed.ciphertext);
    for (index, chunk) in encoded.as_bytes().chunks(SEALED_CHUNK_CHARS).enumerate() {
        properties.insert(
            format!("c{index}"),
            String::from_utf8_lossy(chunk).into_owned(),
        );
    }
    properties
}

fn sealed_from(info: &mdns_sd::ResolvedService) -> Option<SealedLanProof> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let nonce = URL_SAFE_NO_PAD
        .decode(info.get_property_val_str("n")?)
        .ok()?;
    let nonce: [u8; SEAL_NONCE_BYTES] = nonce.try_into().ok()?;
    let mut encoded = String::new();
    for index in 0..MAX_SEALED_CHUNKS {
        let Some(chunk) = info.get_property_val_str(&format!("c{index}")) else {
            break;
        };
        if chunk.len() > SEALED_CHUNK_CHARS {
            return None;
        }
        encoded.push_str(chunk);
    }
    let ciphertext = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    if ciphertext.is_empty() || ciphertext.len() > super::lan_privacy::MAX_SEALED_BYTES {
        return None;
    }
    Some(SealedLanProof { nonce, ciphertext })
}

fn sighting_from(info: &mdns_sd::ResolvedService) -> Option<LanSighting> {
    let id = info
        .get_property_val_str("id")
        .map(str::to_string)
        .or_else(|| id_from_fullname(info.get_fullname()))?;
    if !valid_id(&id) {
        return None;
    }
    let port = |key: &str| {
        info.get_property_val_str(key)
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(0)
    };
    let p4 = port("p4");
    let p6 = port("p6");
    if p4 == 0 && p6 == 0 {
        return None;
    }
    let uplink = info.get_property_val_str("up") == Some("1");
    // Resolved addresses are `ScopedIp` (address plus interface/scope);
    // compare and store the plain address, deduplicated as before.
    let mut addrs: Vec<std::net::IpAddr> = info
        .get_addresses()
        .iter()
        .map(mdns_sd::ScopedIp::to_ip_addr)
        .collect();
    addrs.sort();
    addrs.dedup();
    addrs.truncate(MAX_ADDRESSES);
    Some(LanSighting {
        id,
        addrs,
        p4,
        p6,
        uplink,
        seen_at: super::core::now_secs(),
    })
}

fn event_from(info: &mdns_sd::ResolvedService) -> Option<LanEvent> {
    let mut sighting = sighting_from(info)?;
    let version = info.get_property_val_str("v");
    if !matches!(version, Some("2" | "3")) {
        return Some(LanEvent::Seen(sighting));
    }
    if sighting.id.len() != 32 {
        return None;
    }
    if version == Some("3") {
        // The resolved addresses stay as received: the authenticator opens
        // the seal and accepts the sighting only when every one of them is
        // in the signed list, so an injected address never passes.
        return Some(LanEvent::SeenAuthenticated {
            proof: LanProof {
                epoch: info.get_property_val_str("e")?.parse().ok()?,
                expires_at: info.get_property_val_str("x")?.parse().ok()?,
                addresses: Vec::new(),
                signature: String::new(),
                sealed: Some(sealed_from(info)?),
            },
            sighting,
        });
    }
    let mut addresses = Vec::new();
    for index in 0..MAX_ADDRESSES {
        let Some(value) = info.get_property_val_str(&format!("a{index}")) else {
            break;
        };
        addresses.push(value.parse().ok()?);
    }
    // Never attribute an authenticated advisory to addresses injected into
    // an unsigned mDNS record. Only the signed, actually resolved addresses
    // can later identify a private link.
    sighting.addrs.retain(|address| addresses.contains(address));
    let signature = info.get_property_val_str("s")?;
    if signature.len() > 128 {
        return None;
    }
    Some(LanEvent::SeenAuthenticated {
        sighting,
        proof: LanProof {
            epoch: info.get_property_val_str("e")?.parse().ok()?,
            expires_at: info.get_property_val_str("x")?.parse().ok()?,
            addresses,
            signature: signature.to_string(),
            sealed: None,
        },
    })
}

fn id_from_fullname(fullname: &str) -> Option<String> {
    let instance = fullname.split('.').next()?;
    let id = instance
        .strip_prefix(SEALED_INSTANCE_PREFIX)
        .or_else(|| instance.strip_prefix(INSTANCE_PREFIX))?;
    valid_id(id).then(|| id.to_string())
}

fn valid_id(id: &str) -> bool {
    matches!(id.len(), 16 | 32) && id.bytes().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_task_mdns_interface_check_runs_every_30_seconds() {
        let daemon = mdns_sd::ServiceDaemon::new().expect("mDNS daemon starts without a network");
        configure_daemon(&daemon).expect("options are accepted");
        assert_eq!(IP_CHECK_INTERVAL_SECS, 30);
        assert_eq!(
            daemon
                .get_ip_check_interval()
                .expect("interval is readable"),
            30
        );
        let _ = daemon.shutdown();
    }

    #[test]
    fn lan_cleanup_task_instance_names_carry_the_hashed_id() {
        assert_eq!(
            id_from_fullname("se-0123456789abcdef._se-share._udp.local."),
            Some("0123456789abcdef".to_string())
        );
        assert!(id_from_fullname("other._se-share._udp.local.").is_none());
        assert!(id_from_fullname("se-short._se-share._udp.local.").is_none());
        assert!(
            id_from_fullname("se-0123456789abcdef0123456789abcdef._se-share._udp.local.").is_some()
        );
    }
}

#[cfg(test)]
#[path = "lan_presence_task_tests.rs"]
mod task_tests;
