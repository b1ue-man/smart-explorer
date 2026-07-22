use super::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const PEER_CONNECTIONS_ROOT: &str = "/Verbindungen";
static PEER_CAPABILITY_PROBE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(super) struct PeerMountDiscovery {
    pub(super) open_target: crate::share::PeerOpenTarget,
    pub(super) mount_target: crate::mount::PeerMountTarget,
    pub(super) label: String,
    pub(super) roots: Vec<PeerRootChoice>,
    pub(super) selected: usize,
    pub(super) safety: Result<crate::vfs::MountPathCapabilities, String>,
}

pub(super) struct PeerDraft {
    pub(super) open_target: crate::share::PeerOpenTarget,
    pub(super) roots: Vec<PeerRootChoice>,
    pub(super) selected: usize,
    pub(super) safety: Option<Result<crate::vfs::MountPathCapabilities, String>>,
    pub(super) probe_rx: Option<Receiver<PeerSafetyProbe>>,
}

#[derive(Clone)]
pub(super) struct PeerRootChoice {
    pub(super) root: crate::mount::BackendRoot,
    pub(super) label: String,
}

pub(super) struct PeerSafetyProbe {
    root: crate::mount::BackendRoot,
    safety: Result<crate::vfs::MountPathCapabilities, String>,
}

pub(super) fn discover_peer_mount(
    open_target: crate::share::PeerOpenTarget,
    label: String,
) -> Result<PeerMountDiscovery, String> {
    let mount_target = peer_mount_target(&open_target);
    let (_, backend, _) = crate::daemon::open_share_backend(open_target.clone())?;
    let roots = peer_roots(&backend)?;
    let selected = roots
        .iter()
        .position(|choice| choice.root.as_str() != "/")
        .unwrap_or(0);
    let selected_root = roots
        .get(selected)
        .ok_or_else(|| "Der Peer hat keine einbindbare Freigabe gemeldet".to_string())?;
    let safety = probe_peer_safety(open_target.clone(), &selected_root.root);
    Ok(PeerMountDiscovery {
        open_target,
        mount_target,
        label,
        roots,
        selected,
        safety,
    })
}

pub(super) fn begin_peer_probe(peer: &mut PeerDraft, ctx: &egui::Context) {
    let target = peer.open_target.clone();
    let root = peer.roots[peer.selected].root.clone();
    let permit = match PeerProbePermit::acquire() {
        Ok(permit) => permit,
        Err(error) => {
            peer.safety = Some(Err(error));
            peer.probe_rx = None;
            return;
        }
    };
    let (sender, receiver) = unbounded();
    match std::thread::Builder::new()
        .name("peer-mount-capability".into())
        .spawn(move || {
            let _permit = permit;
            let safety = crate::daemon::probe_share_mount_capabilities(target, &root);
            let _ = sender.send(PeerSafetyProbe { root, safety });
        }) {
        Ok(_) => {
            peer.safety = None;
            peer.probe_rx = Some(receiver);
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        Err(error) => {
            peer.safety = Some(Err(format!("Peer-Pruefung starten: {error}")));
            peer.probe_rx = None;
        }
    }
}

fn probe_peer_safety(
    target: crate::share::PeerOpenTarget,
    root: &crate::mount::BackendRoot,
) -> Result<crate::vfs::MountPathCapabilities, String> {
    let _permit = PeerProbePermit::acquire()?;
    crate::daemon::probe_share_mount_capabilities(target, root)
}

struct PeerProbePermit;

impl PeerProbePermit {
    fn acquire() -> Result<Self, String> {
        PEER_CAPABILITY_PROBE_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| {
                "Eine vorherige Peer-Laufwerkspruefung laeuft noch; bitte danach erneut pruefen"
                    .into()
            })
    }
}

impl Drop for PeerProbePermit {
    fn drop(&mut self) {
        PEER_CAPABILITY_PROBE_ACTIVE.store(false, Ordering::Release);
    }
}

pub(super) fn poll_peer_probe(draft: &mut super::mount_ui_draft::MountDraft, ctx: &egui::Context) {
    let Some(peer) = draft.peer.as_mut() else {
        return;
    };
    let Some(receiver) = peer.probe_rx.as_ref() else {
        return;
    };
    match receiver.try_recv() {
        Ok(probe) => {
            if probe.root == peer.roots[peer.selected].root {
                peer.safety = Some(probe.safety);
            }
            peer.probe_rx = None;
            ctx.request_repaint();
        }
        Err(crossbeam_channel::TryRecvError::Disconnected) => {
            peer.safety = Some(Err("Peer-Pruefung wurde ohne Ergebnis beendet".into()));
            peer.probe_rx = None;
        }
        Err(crossbeam_channel::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }
}

fn peer_roots(backend: &crate::vfs::BackendHandle) -> Result<Vec<PeerRootChoice>, String> {
    let mut listings = Vec::new();
    for entry in backend.list_dir("/").map_err(|error| error.to_string())? {
        if !entry.is_dir || entry.is_symlink {
            continue;
        }
        if entry.name == PEER_CONNECTIONS_ROOT.trim_start_matches('/') {
            let Ok(connections) = backend.list_dir(PEER_CONNECTIONS_ROOT) else {
                // Older peers could accidentally advertise a colliding local
                // label here. Do not let one unusable reserved entry hide all
                // other concrete mount roots during discovery.
                continue;
            };
            let connections = connections
                .into_iter()
                .filter(|connection| connection.is_dir && !connection.is_symlink)
                .map(|connection| connection.name)
                .collect();
            listings.push(PeerRootListing {
                name: entry.name,
                connections: Some(connections),
            });
        } else {
            listings.push(PeerRootListing {
                name: entry.name,
                connections: None,
            });
        }
    }
    Ok(build_peer_roots(listings))
}

struct PeerRootListing {
    name: String,
    connections: Option<Vec<String>>,
}

fn build_peer_roots(listings: Vec<PeerRootListing>) -> Vec<PeerRootChoice> {
    let mut roots = Vec::new();
    for listing in listings {
        if !safe_virtual_segment(&listing.name) {
            continue;
        }
        if let Some(connections) = listing.connections {
            for connection in connections {
                if safe_virtual_segment(&connection) {
                    push_root(
                        &mut roots,
                        &format!("{PEER_CONNECTIONS_ROOT}/{connection}"),
                        format!("Verbindung: {connection}"),
                    );
                }
            }
        } else {
            push_root(&mut roots, &format!("/{}", listing.name), listing.name);
        }
    }
    push_root(
        &mut roots,
        "/",
        "Alle freigegebenen Bereiche (nur Lesen)".into(),
    );
    roots
}

fn push_root(roots: &mut Vec<PeerRootChoice>, path: &str, label: String) {
    let Ok(root) = crate::mount::BackendRoot::parse(path) else {
        return;
    };
    if roots.iter().any(|choice| choice.root == root) {
        return;
    }
    roots.push(PeerRootChoice { root, label });
}

fn safe_virtual_segment(value: &str) -> bool {
    !value.is_empty()
        && value.encode_utf16().count() <= 255
        && !matches!(value, "." | "..")
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
}

fn peer_mount_target(target: &crate::share::PeerOpenTarget) -> crate::mount::PeerMountTarget {
    match target {
        crate::share::PeerOpenTarget::Direct { contact_id } => {
            crate::mount::PeerMountTarget::Direct {
                contact_id: contact_id.clone(),
            }
        }
        crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
            crate::mount::PeerMountTarget::RoomDevice {
                room_id: room_id.clone(),
                device_id: device_id.clone(),
            }
        }
    }
}

#[cfg(test)]
pub(super) fn peer_root_paths_for_test(entries: &[(&str, &[&str])]) -> Vec<String> {
    build_peer_roots(
        entries
            .iter()
            .map(|(name, children)| PeerRootListing {
                name: (*name).to_string(),
                connections: (*name == PEER_CONNECTIONS_ROOT.trim_start_matches('/'))
                    .then(|| children.iter().map(|child| (*child).to_string()).collect()),
            })
            .collect(),
    )
    .into_iter()
    .map(|choice| choice.root.as_str().to_string())
    .collect()
}

#[cfg(test)]
#[path = "mount_peer_roots_task_tests.rs"]
mod task_tests;
