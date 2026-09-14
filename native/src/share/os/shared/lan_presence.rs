//! mDNS announcement and discovery of Smart Explorer Direct endpoints on the
//! local network (`_se-share._udp.local.`).
//!
//! The announcement carries a hashed endpoint id, the Iroh UDP ports and an
//! advisory uplink flag; matching and authentication happen elsewhere. The
//! daemon starts this once; failures are reported as explicit errors and the
//! owner retries later instead of assuming multicast works.
use std::collections::HashMap;
use std::sync::Mutex;

use crossbeam_channel::{unbounded, Receiver};

use super::lan_presence_match::{LanSighting, LAN_PRESENCE_VERSION};

const SERVICE: &str = "_se-share._udp.local.";
const INSTANCE_PREFIX: &str = "se-";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanAnnouncement {
    pub hashed_id: String,
    pub p4: u16,
    pub p6: u16,
    pub uplink: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanEvent {
    Seen(LanSighting),
    Lost(String),
    Error(String),
}

/// Live mDNS browser/announcer. Dropping it stops both.
pub struct LanPresence {
    events: Receiver<LanEvent>,
    daemon: mdns_sd::ServiceDaemon,
    announced: Mutex<Option<(String, LanAnnouncement)>>,
}

impl LanPresence {
    pub fn start() -> Result<Self, String> {
        let daemon = mdns_sd::ServiceDaemon::new()
            .map_err(|error| format!("mDNS-Dienst konnte nicht gestartet werden: {error}"))?;
        let browse = daemon
            .browse(SERVICE)
            .map_err(|error| format!("mDNS-Suche konnte nicht gestartet werden: {error}"))?;
        let (tx, events) = unbounded();
        std::thread::Builder::new()
            .name("se-lan-presence".into())
            .spawn(move || {
                while let Ok(event) = browse.recv() {
                    let mapped = match event {
                        mdns_sd::ServiceEvent::ServiceResolved(info) => {
                            sighting_from(&info).map(LanEvent::Seen)
                        }
                        mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                            id_from_fullname(&fullname).map(LanEvent::Lost)
                        }
                        _ => None,
                    };
                    if let Some(mapped) = mapped {
                        if tx.send(mapped).is_err() {
                            break;
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
        self.announced
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|(_, announcement)| announcement.clone()))
    }

    /// Register (or replace) our own announcement. Unchanged announcements
    /// are a no-op.
    pub fn announce(&self, announcement: &LanAnnouncement) -> Result<(), String> {
        let mut guard = self
            .announced
            .lock()
            .map_err(|_| "mDNS-Ankuendigung ist gesperrt".to_string())?;
        if guard.as_ref().is_some_and(|(_, current)| current == announcement) {
            return Ok(());
        }
        if let Some((fullname, _)) = guard.take() {
            let _ = self.daemon.unregister(&fullname);
        }
        let instance = format!("{INSTANCE_PREFIX}{}", announcement.hashed_id);
        let host = format!("{}.local.", sanitize(&hostname()));
        let properties: HashMap<String, String> = HashMap::from([
            ("v".to_string(), LAN_PRESENCE_VERSION.to_string()),
            ("id".to_string(), announcement.hashed_id.clone()),
            ("p4".to_string(), announcement.p4.to_string()),
            ("p6".to_string(), announcement.p6.to_string()),
            ("up".to_string(), u8::from(announcement.uplink).to_string()),
        ]);
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
        *guard = Some((fullname, announcement.clone()));
        Ok(())
    }

    pub fn withdraw(&self) {
        if let Ok(mut guard) = self.announced.lock() {
            if let Some((fullname, _)) = guard.take() {
                let _ = self.daemon.unregister(&fullname);
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

fn sighting_from(info: &mdns_sd::ServiceInfo) -> Option<LanSighting> {
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
    let mut addrs: Vec<std::net::IpAddr> = info.get_addresses().iter().copied().collect();
    addrs.sort();
    Some(LanSighting {
        id,
        addrs,
        p4,
        p6,
        uplink,
        seen_at: super::core::now_secs(),
    })
}

fn id_from_fullname(fullname: &str) -> Option<String> {
    let instance = fullname.split('.').next()?;
    let id = instance.strip_prefix(INSTANCE_PREFIX)?;
    valid_id(id).then(|| id.to_string())
}

fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "smart-explorer".to_string())
}

fn sanitize(value: &str) -> String {
    let out: String = value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    if out.trim_matches('-').is_empty() {
        "smart-explorer".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names_carry_the_hashed_id() {
        assert_eq!(
            id_from_fullname("se-0123456789abcdef._se-share._udp.local."),
            Some("0123456789abcdef".to_string())
        );
        assert!(id_from_fullname("other._se-share._udp.local.").is_none());
        assert!(id_from_fullname("se-short._se-share._udp.local.").is_none());
        assert_eq!(sanitize("My Host!"), "My-Host-");
        assert_eq!(sanitize("!!!"), "smart-explorer");
    }
}
