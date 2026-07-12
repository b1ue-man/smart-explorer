use crossbeam_channel::{bounded, unbounded, Receiver};
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::backend::{PeerBackend, ShareIrohNode};
use super::core::eio;
use super::identity::ShareIdentity;
use super::profiles::ShareProfiles;
use super::types::{
    PeerEndpoint, PeerOpenTarget, PendingShareCmd, ShareAuthState, ShareCmd, ShareEvent,
    ShareScope, ShareStatus,
};

pub struct ShareService {
    pub events: Receiver<ShareEvent>,
    pub(super) cmds: super::types::CmdTx,
    pub identity: ShareIdentity,
    pub listen_port: u16,
    pub(super) auth: Arc<Mutex<ShareAuthState>>,
    pub(super) iroh: Arc<ShareIrohNode>,
    pub(super) stopped: Arc<AtomicBool>,
    pub(super) server: String,
    pub(super) owner: bool,
}

impl ShareService {
    pub fn cmd(&self, c: ShareCmd) -> Result<(), String> {
        const ACK_TIMEOUT: Duration = Duration::from_secs(5);
        let is_stop = matches!(&c, ShareCmd::Stop);
        let (acknowledge, acknowledged) = bounded(1);
        let expires_at = Instant::now() + ACK_TIMEOUT;
        let send_result = self.cmds.send(PendingShareCmd {
            command: c,
            acknowledgement: acknowledge,
            expires_at,
        });
        let result = match send_result {
            Ok(()) => match acknowledged.recv_timeout(ACK_TIMEOUT) {
                Ok(result) => result,
                Err(error) => Err(format!("Share-Kommando nicht bestaetigt: {error}")),
            },
            Err(error) => Err(format!(
                "Share-Kommando konnte nicht zugestellt werden: {error}"
            )),
        };
        if is_stop {
            self.stopped.store(true, Ordering::Relaxed);
        }
        result
    }

    pub fn probe_backend_for_target(
        &self,
        target: &PeerOpenTarget,
    ) -> Result<(String, crate::vfs::BackendHandle, ShareStatus), String> {
        let endpoint = self.endpoint_for_target(target)?;
        let label = endpoint.label.clone();
        let be = PeerBackend::new(endpoint, self.identity.clone(), self.iroh.clone());
        be.probe_root().map_err(|e| e.to_string())?;
        let status = be.transport_status();
        Ok((label, Arc::new(be), status))
    }

    pub fn exec_for_target(
        &self,
        target: &PeerOpenTarget,
        req: super::types::ExecRequest,
    ) -> Result<super::types::ExecResult, String> {
        let endpoint = self.endpoint_for_target(target)?;
        let be = PeerBackend::new(endpoint, self.identity.clone(), self.iroh.clone());
        be.exec(req).map_err(|e| e.to_string())
    }

    pub fn relay_url(&self) -> String {
        self.iroh.relay_url().to_string()
    }

    pub fn peer_candidates(&self) -> Vec<String> {
        self.iroh.candidates()
    }

    fn endpoint_for_target(&self, target: &PeerOpenTarget) -> Result<PeerEndpoint, String> {
        let state = self
            .auth
            .lock()
            .map_err(|_| "Share-State gesperrt")?
            .clone();
        match target {
            PeerOpenTarget::Direct { contact_id } => {
                let contact = state
                    .direct_contacts
                    .iter()
                    .find(|c| &c.id == contact_id)
                    .ok_or_else(|| "Direktgeraet nicht gefunden".to_string())?;
                let presence = contact
                    .presence
                    .clone()
                    .ok_or_else(|| "Direktgeraet ist nicht online".to_string())?;
                let secret = ShareProfiles::direct_secret_checked(contact)?
                    .ok_or_else(|| "Direkt-Secret fehlt".to_string())?;
                let expected_node_id = if contact.expected_node_id.trim().is_empty() {
                    Some(presence.node_id.clone())
                } else {
                    Some(contact.expected_node_id.clone())
                };
                Ok(PeerEndpoint {
                    label: format!("Share Direkt: {}", contact.display_name),
                    scope: ShareScope::Direct {
                        contact_id: contact.id.clone(),
                    },
                    presence,
                    relation_secret: secret,
                    expected_node_id,
                })
            }
            PeerOpenTarget::RoomDevice { room_id, device_id } => {
                let room = state
                    .rooms
                    .iter()
                    .find(|r| &r.id == room_id || &r.room_id == room_id)
                    .ok_or_else(|| "Raum nicht gefunden".to_string())?;
                let member = room
                    .members
                    .iter()
                    .find(|m| &m.device_id == device_id)
                    .ok_or_else(|| "Geraet nicht im Raum".to_string())?;
                if member.blocked {
                    return Err("Geraet ist blockiert".into());
                }
                let presence = member
                    .presence
                    .clone()
                    .ok_or_else(|| "Raumgeraet ist nicht online".to_string())?;
                let secret = ShareProfiles::room_secret_checked(room)?
                    .ok_or_else(|| "Raum-Secret fehlt".to_string())?;
                Ok(PeerEndpoint {
                    label: format!("Share Raum {} / {}", room.name, member.device_name),
                    scope: ShareScope::Room {
                        room_id: room.room_id.clone(),
                    },
                    presence,
                    relation_secret: secret,
                    expected_node_id: Some(member.node_id.clone()),
                })
            }
        }
    }

    pub fn start(
        server: String,
        identity: ShareIdentity,
        profiles: ShareProfiles,
    ) -> io::Result<ShareService> {
        let listen_port = 0;
        let (cmd_tx, cmd_rx) = unbounded::<PendingShareCmd>();
        let (ev_tx, ev_rx) = unbounded::<ShareEvent>();
        let stopped = Arc::new(AtomicBool::new(false));
        match super::system::ensure_firewall_rule() {
            Ok(msg) => {
                let _ = ev_tx.send(ShareEvent::Status(msg));
            }
            Err(e) => {
                let _ = ev_tx.send(ShareEvent::Status(format!(
                    "Firewall-Regel fuer Peer-Listener nicht gesetzt: {e}"
                )));
            }
        }

        let auth = Arc::new(Mutex::new(ShareAuthState {
            direct_secret: identity.direct_secret(),
            identity: identity.clone(),
            default_direct_exports: profiles.default_direct_exports.clone(),
            direct_contacts: profiles.direct_contacts.clone(),
            direct_grants: profiles.direct_grants.clone(),
            rooms: profiles.rooms.clone(),
            direct_requests: profiles.direct_requests.clone(),
            seen_nonces: HashSet::new(),
            direct_online: true,
        }));

        let iroh = ShareIrohNode::start(&server, &identity, auth.clone(), ev_tx.clone())?;
        let _ = ev_tx.send(ShareEvent::Status(format!(
            "Iroh bereit: node={}, relay={}",
            identity.node_id,
            iroh.relay_url()
        )));

        {
            let auth = auth.clone();
            let ev = ev_tx.clone();
            let identity_worker = identity.clone();
            let iroh_worker = iroh.clone();
            let stopped = stopped.clone();
            let worker_server = server.clone();
            std::thread::Builder::new()
                .name("share-signal".into())
                .spawn(move || {
                    super::signal_worker::worker(
                        worker_server,
                        identity_worker,
                        iroh_worker,
                        auth,
                        cmd_rx,
                        ev,
                        stopped,
                    )
                })
                .map_err(eio)?;
        }

        Ok(ShareService {
            events: ev_rx,
            cmds: cmd_tx,
            identity,
            listen_port,
            auth,
            iroh,
            stopped,
            server,
            owner: true,
        })
    }
}

impl Clone for ShareService {
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
            cmds: self.cmds.clone(),
            identity: self.identity.clone(),
            listen_port: self.listen_port,
            auth: self.auth.clone(),
            iroh: self.iroh.clone(),
            stopped: self.stopped.clone(),
            server: self.server.clone(),
            owner: false,
        }
    }
}

impl Drop for ShareService {
    fn drop(&mut self) {
        if self.owner {
            self.stopped.store(true, Ordering::Relaxed);
        }
    }
}
