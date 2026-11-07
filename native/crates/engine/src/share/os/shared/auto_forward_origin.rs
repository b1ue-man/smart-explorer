//! Origin rules of the viewer's listeners (design 2.7.1 `Process`, 4.4
//! `SameAccount`): which local process may use a mapped host service.
use crate::share::os_sockets;
use crate::share::tcp_registry::{BoundProcess, OriginCheck};
use std::net::SocketAddr;

/// The socket-owner facts of this OS behind the origin rules of design
/// 2.7.1 (`Process`) and 4.4 (`SameAccount`).
pub struct SocketOwnerCheck {
    own: String,
}
impl SocketOwnerCheck {
    pub fn new() -> Result<Self, String> {
        let own = os_sockets::own_account()
            .map_err(|error| format!("Eigenes Konto nicht ermittelbar: {error}"))?;
        Ok(Self { own })
    }
}

/// The UID of the active session on `seat0` (logind). A root background
/// service (servers) accepts that person's processes besides its own UID
/// (design 4.4). Only consulted when this service runs as UID 0, which
/// exists on Linux alone; the file is absent everywhere else.
fn active_seat_uid() -> Option<String> {
    let text = std::fs::read_to_string("/run/systemd/seats/seat0").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("ACTIVE_UID="))
        .map(|uid| uid.trim().to_string())
        .filter(|uid| !uid.is_empty() && uid.bytes().all(|byte| byte.is_ascii_digit()))
}

impl OriginCheck for SocketOwnerCheck {
    fn same_account(&self, peer: SocketAddr, listener: SocketAddr) -> Result<(), String> {
        let account = os_sockets::connection_account(peer, listener)
            .map_err(|error| {
                format!("Herkunft der Verbindung nicht ermittelbar ({error}); abgewiesen")
            })?
            .ok_or_else(|| "Herkunft der Verbindung nicht ermittelbar; abgewiesen".to_string())?;
        let seat = (self.own == "0").then(active_seat_uid).flatten();
        if account == self.own || seat.as_deref() == Some(account.as_str()) {
            Ok(())
        } else {
            Err(format!(
                "Verbindung eines anderen Kontos abgewiesen (Konto {account})"
            ))
        }
    }

    fn process(
        &self,
        peer: SocketAddr,
        listener: SocketAddr,
        bound: BoundProcess,
    ) -> Result<(), String> {
        // Only the bound process's own sockets are read (review R3): a flood
        // of foreign connections costs no walk over every process.
        let pid = bound.pid;
        let owned = os_sockets::connection_owned_by(peer, listener, pid).map_err(|error| {
            format!("Herkunft der Verbindung nicht ermittelbar ({error}); abgewiesen")
        })?;
        if !owned {
            return Err(format!(
                "Verbindung eines fremden Prozesses abgewiesen (nicht PID {pid})"
            ));
        }
        let identity = os_sockets::process_identity(pid)
            .map_err(|error| format!("Prozess {pid} nicht prüfbar ({error}); abgewiesen"))?;
        if identity.started != bound.started {
            return Err(format!(
                "Verbindung eines fremden Prozesses abgewiesen (PID {pid}, anderer Prozessstart)"
            ));
        }
        Ok(())
    }
}
