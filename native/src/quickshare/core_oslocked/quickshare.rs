//! Quick Share (Android "Nearby Share") — LAN discovery (#Q).
//!
//! Quick Share devices announce themselves on the local network via mDNS under
//! the service type `_FC9F5ED42C8A._tcp`. This module browses that service (and
//! advertises ours) so nearby Android/Windows Quick Share endpoints show up in
//! the 📡 Teilen view.
//!
//! The actual **file transfer** to/from Quick Share is the large remaining
//! piece — it needs the Nearby Connections **UKEY2** handshake + **protobuf**
//! `OfflineFrame` payloads (and BLE to wake Android's "Everyone" visibility).
//! That layer wants real-device iteration; see `docs/QUICKSHARE.md`. The own
//! paired share (📡 Teilen) already provides cross-device transfer today.
//!
//! Discovery is pure Rust (`mdns-sd`) and runs only while the Teilen view is
//! open, so it adds no idle overhead.

use crossbeam_channel::{unbounded, Receiver};
use std::collections::BTreeMap;

const SERVICE: &str = "_FC9F5ED42C8A._tcp.local.";

/// A Quick Share endpoint seen on the LAN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QsDevice {
    pub name: String,
    pub addr: String,
}

/// Live Quick Share discovery. Drop to stop browsing/advertising.
pub struct QuickShare {
    /// Latest full device list (replaced on each change).
    pub events: Receiver<Vec<QsDevice>>,
    _daemon: mdns_sd::ServiceDaemon,
}

impl QuickShare {
    /// Start browsing (and advertising) the Quick Share mDNS service. Returns
    /// None if mDNS can't be initialised.
    pub fn start(my_name: &str) -> Option<QuickShare> {
        let daemon = mdns_sd::ServiceDaemon::new().ok()?;

        // Advertise ourselves so Android can see this PC (best-effort).
        if let Ok(host) = hostname_local() {
            if let Ok(svc) = mdns_sd::ServiceInfo::new(
                SERVICE,
                &sanitize_instance(my_name),
                &host,
                "",
                0u16,
                None,
            ) {
                let _ = daemon.register(svc);
            }
        }

        let browse = match daemon.browse(SERVICE) {
            Ok(rx) => rx,
            Err(_) => return None,
        };
        let (tx, events) = unbounded();
        std::thread::Builder::new()
            .name("quickshare-mdns".into())
            .spawn(move || {
                // Keyed by fullname so add/remove keep the list consistent.
                let mut devices: BTreeMap<String, QsDevice> = BTreeMap::new();
                while let Ok(ev) = browse.recv() {
                    match ev {
                        mdns_sd::ServiceEvent::ServiceResolved(info) => {
                            let name = pretty_name(info.get_fullname());
                            let addr = info
                                .get_addresses()
                                .iter()
                                .next()
                                .map(|a| format!("{}:{}", a, info.get_port()))
                                .unwrap_or_default();
                            devices.insert(
                                info.get_fullname().to_string(),
                                QsDevice { name, addr },
                            );
                            let _ = tx.send(devices.values().cloned().collect());
                        }
                        mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                            devices.remove(&fullname);
                            let _ = tx.send(devices.values().cloned().collect());
                        }
                        _ => {}
                    }
                }
            })
            .ok();
        Some(QuickShare { events, _daemon: daemon })
    }
}

fn hostname_local() -> Result<String, ()> {
    let h = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "smart-explorer".to_string());
    Ok(format!("{}.local.", sanitize_instance(&h)))
}

fn sanitize_instance(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    if out.is_empty() { "smart-explorer".to_string() } else { out }
}

/// Quick Share advertises an obfuscated instance name; show the readable head.
fn pretty_name(fullname: &str) -> String {
    let head = fullname.split('.').next().unwrap_or(fullname);
    if head.is_empty() {
        fullname.to_string()
    } else {
        head.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_and_pretty() {
        assert_eq!(sanitize_instance("My PC!"), "My-PC-");
        assert_eq!(pretty_name("abcd1234._FC9F5ED42C8A._tcp.local."), "abcd1234");
    }
}
