//! `UplinkAdapter` for Linux: NetworkManager's `shared` IPv4 method over
//! D-Bus, with `dnsmasq` and polkit as probed prerequisites.
use std::time::{Duration, Instant};

use crate::net::{Facility, InterfaceFacts, UplinkAdapter, UplinkTarget};

use super::{nm_shared, uplink_polkit};

const PROBE_CACHE: Duration = Duration::from_secs(30);

pub(crate) struct NetworkManagerAdapter {
    probed_at: Option<Instant>,
    facility: Facility,
}

impl NetworkManagerAdapter {
    pub(crate) fn new() -> Self {
        Self {
            probed_at: None,
            facility: Facility::Unavailable("noch nicht geprueft".into()),
        }
    }

    fn probe_now() -> Facility {
        let connection = match nm_shared::connect() {
            Ok(connection) => connection,
            Err(error) => return Facility::Unavailable(error.to_string()),
        };
        match nm_shared::available(&connection) {
            Ok(true) => {}
            Ok(false) => {
                return Facility::Unavailable("NetworkManager laeuft nicht".into());
            }
            Err(error) => return Facility::Unavailable(format!("NetworkManager: {error}")),
        }
        if !dnsmasq_available() {
            return Facility::Unavailable(
                "dnsmasq fehlt (NetworkManager braucht es fuer geteilte Verbindungen)".into(),
            );
        }
        Facility::Available
    }
}

fn dnsmasq_available() -> bool {
    let from_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("dnsmasq").is_file()));
    from_path
        || ["/usr/sbin/dnsmasq", "/sbin/dnsmasq", "/usr/bin/dnsmasq"]
            .iter()
            .any(|candidate| std::path::Path::new(candidate).is_file())
}

impl UplinkAdapter for NetworkManagerAdapter {
    fn probe(&mut self, _setup_done: bool) -> Facility {
        let fresh = self.probed_at.is_some_and(|at| at.elapsed() < PROBE_CACHE);
        if !fresh {
            self.facility = Self::probe_now();
            self.probed_at = Some(Instant::now());
        }
        self.facility.clone()
    }

    fn setup_once(&mut self) -> Result<String, String> {
        self.probed_at = None;
        if uplink_polkit::rule_installed() {
            return Ok("polkit-Regel ist bereits installiert".into());
        }
        match uplink_polkit::install_rule() {
            Ok(message) => Ok(message),
            Err(error) if !uplink_polkit::pkexec_available() => Ok(format!(
                "{error}; NetworkManager wird ohne Regel angesprochen und fragt ggf. ueber den Desktop nach"
            )),
            Err(error) => Err(error.to_string()),
        }
    }

    fn enable(&mut self, private: &UplinkTarget, _public: &UplinkTarget) -> Result<(), String> {
        let connection = nm_shared::connect().map_err(|error| error.to_string())?;
        nm_shared::enable_shared(&connection, &private.adapter_id)
            .map_err(|error| error.to_string())
    }

    fn disable(&mut self, private: &UplinkTarget, _public: &UplinkTarget) -> Result<(), String> {
        let connection = nm_shared::connect().map_err(|error| error.to_string())?;
        nm_shared::disable_shared(&connection, &private.adapter_id)
            .map_err(|error| error.to_string())
    }

    fn sharing_active(&mut self, private: &UplinkTarget) -> Result<Option<bool>, String> {
        let connection = nm_shared::connect().map_err(|error| error.to_string())?;
        nm_shared::shared_active(&connection, &private.adapter_id)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn internet_ifaces(&mut self, facts: &[InterfaceFacts]) -> Option<Vec<u32>> {
        let connection = nm_shared::connect().ok()?;
        if !nm_shared::connectivity_full(&connection).ok()? {
            return Some(Vec::new());
        }
        // NetworkManager reports connectivity per host; attribute it to the
        // interfaces that carry a gateway according to NM.
        let mut out = Vec::new();
        for iface in facts {
            if !iface.up || iface.loopback {
                continue;
            }
            match nm_shared::ip4_gateway(&connection, &iface.name) {
                Ok(Some(gateway)) if !gateway.is_empty() => out.push(iface.index),
                Ok(_) => {}
                Err(_) => {
                    if iface.has_gateway {
                        out.push(iface.index);
                    }
                }
            }
        }
        Some(out)
    }

    fn refine_facts(&mut self, facts: &mut [InterfaceFacts]) {
        let Ok(connection) = nm_shared::connect() else {
            return;
        };
        if !nm_shared::available(&connection).unwrap_or(false) {
            return;
        }
        for iface in facts.iter_mut() {
            if iface.loopback {
                continue;
            }
            if let Ok(Some(lease)) = nm_shared::dhcp_lease(&connection, &iface.name) {
                iface.dhcp_lease = Some(lease);
            }
        }
    }
}
