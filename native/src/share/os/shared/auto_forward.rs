//! Viewer side of „localhost is shared“ (design 4.3/4.4, E12, user rule
//! r5): every service a host shares under „Bildschirm steuern: Erlauben“
//! appears on this device at `<device address>:<same port>`. The listener
//! exists while the device feed lists the service; the QUIC tunnel opens
//! with the first admitted local connection and ends with the last one.
//! Desktop listeners admit only processes of this service's own account
//! (`SameAccount`); Android cannot tell who connects (assumption A7) and
//! therefore opens a listener only on the person's tap (`AnyLocal`).
use crate::share::device_address::device_loopback;
use crate::share::loopback_services::LoopbackService;
use crate::share::os_sockets;
use crate::share::tcp_forward::{ListenerAdmission, TcpDestination, TcpStart, TunnelId};
use crate::share::tcp_registry::{BoundProcess, OriginCheck};
use crate::share::{PeerOpenTarget, ShareService};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// The socket-owner facts of this OS behind the origin rules of design
/// 2.7.1 (`Process`) and 4.4 (`SameAccount`).
pub(crate) struct SocketOwnerCheck {
    own: String,
}
impl SocketOwnerCheck {
    pub(crate) fn new() -> Result<Self, String> {
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

/// One host service as this device reaches it (design 4.3 „Anzeige“).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct MappedService {
    pub(crate) host_port: u16,
    pub(crate) process: Option<String>,
    /// `<device address>:<port>` while a listener exists.
    pub(crate) address: Option<SocketAddr>,
    /// Why the local port differs from the host port („Port belegt“).
    pub(crate) port_note: Option<String>,
    /// A browser link for the usual web ports.
    pub(crate) url: Option<String>,
    /// The tunnel currently carrying connections, if any.
    pub(crate) tunnel: Option<TunnelId>,
    pub(crate) last_error: Option<String>,
}

/// Design 4.3: a link for 80/443/8000–8999/3000–3999/5000–5999. Port 443
/// carries TLS, so its link names `https`.
pub(crate) fn service_url(address: SocketAddr, host_port: u16) -> Option<String> {
    let web = matches!(host_port, 80 | 443 | 3000..=3999 | 5000..=5999 | 8000..=8999);
    let scheme = if host_port == 443 { "https" } else { "http" };
    web.then(|| format!("{scheme}://{address}"))
}

/// Binds `address:port`; when that port is taken (or not allowed for this
/// account) another free port on the same address, with the reason.
/// Any other failure (for example an address the OS refuses) is an error:
/// a different address would break the device's stable name.
pub(crate) fn bind_mapping(
    address: Ipv4Addr,
    port: u16,
) -> io::Result<(std::net::TcpListener, Option<String>)> {
    let (listener, note) = match std::net::TcpListener::bind((address, port)) {
        Ok(listener) => (listener, None),
        Err(error) => {
            let note = match error.kind() {
                io::ErrorKind::AddrInUse => crate::share::tcp_sockets::taken_note(address, port),
                io::ErrorKind::PermissionDenied => "Port für dieses Konto nicht erlaubt",
                _ => return Err(error),
            };
            (
                std::net::TcpListener::bind((address, 0))?,
                Some(note.to_string()),
            )
        }
    };
    listener.set_nonblocking(true)?;
    Ok((listener, note))
}

struct Mapping {
    view: Arc<Mutex<MappedService>>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Mapping {
    fn stop(&mut self, service: Option<&ShareService>) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let mut view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        if let (Some(id), Some(service)) = (view.tunnel.take(), service) {
            service.tcp_close(&id);
        }
        view.address = None;
        view.url = None;
    }
    fn running(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
}

struct Device {
    service: ShareService,
    target: PeerOpenTarget,
    address: Ipv4Addr,
    mappings: BTreeMap<u16, Mapping>,
}

static DEVICES: OnceLock<Mutex<BTreeMap<String, Device>>> = OnceLock::new();

fn devices() -> std::sync::MutexGuard<'static, BTreeMap<String, Device>> {
    DEVICES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The identity key of one device in this module and `peer_feed`.
pub(crate) fn device_key(device_id: &str, public_key: &str) -> String {
    format!("{device_id}|{public_key}")
}

/// Listeners open without a tap on desktops only (design 4.3 „Zeitpunkt“).
fn automatic() -> bool {
    !cfg!(target_os = "android")
}

fn admission() -> ListenerAdmission {
    if automatic() {
        ListenerAdmission::SameAccount
    } else {
        ListenerAdmission::AnyLocal
    }
}

/// The feed of `device_id` listed `services` (all of them shared with this
/// device). Desktop: listeners for new services start, listeners of gone
/// services stop. Android: the list is kept; listeners open on `open`.
pub(crate) fn apply(
    service: &ShareService,
    device_id: &str,
    public_key: &str,
    target: &PeerOpenTarget,
    services: &[LoopbackService],
) -> Result<(), String> {
    let address = device_loopback(device_id, public_key)
        .map_err(|error| format!("Geräteadresse nicht verfügbar: {error}"))?;
    let key = device_key(device_id, public_key);
    let mut all = devices();
    let stale = all.get(&key).is_some_and(|device| {
        !Arc::ptr_eq(&device.service.iroh, &service.iroh) || device.address != address
    });
    if stale {
        if let Some(device) = all.remove(&key) {
            stop_device(device);
        }
    }
    let device = all.entry(key).or_insert_with(|| Device {
        service: service.clone(),
        target: target.clone(),
        address,
        mappings: BTreeMap::new(),
    });
    device.target = target.clone();
    let listed: BTreeMap<u16, &LoopbackService> =
        services.iter().map(|entry| (entry.port, entry)).collect();
    let gone: Vec<u16> = device
        .mappings
        .keys()
        .filter(|port| !listed.contains_key(port))
        .copied()
        .collect();
    for port in gone {
        if let Some(mut mapping) = device.mappings.remove(&port) {
            mapping.stop(Some(&device.service));
        }
    }
    for (port, entry) in listed {
        let mapping = device.mappings.entry(port).or_insert_with(|| Mapping {
            view: Arc::new(Mutex::new(MappedService {
                host_port: port,
                ..MappedService::default()
            })),
            task: None,
        });
        mapping
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .process = entry.process.clone();
        // A listener whose task ended (accept failure) starts again with
        // the next feed update: the event-driven retry of design 4.3.
        if automatic() && !mapping.running() {
            start(
                &device.service,
                &device.target,
                device.address,
                port,
                mapping,
            );
        }
    }
    Ok(())
}

/// Android tap „Öffnen“ (only the Android bridge calls it).
#[cfg(any(target_os = "android", all(unix, test)))]
pub(crate) fn open(device_id: &str, public_key: &str, port: u16) -> Result<MappedService, String> {
    let mut all = devices();
    let device = all
        .get_mut(&device_key(device_id, public_key))
        .ok_or_else(|| {
            "Gerät liefert keine Dienstliste (offline oder ohne Freigabe)".to_string()
        })?;
    let mapping = device
        .mappings
        .get_mut(&port)
        .ok_or_else(|| format!("Port {port} ist auf dem Gerät nicht freigegeben"))?;
    if !mapping.running() {
        start(
            &device.service,
            &device.target,
            device.address,
            port,
            mapping,
        );
    }
    let view = mapping
        .view
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    match &view.last_error {
        Some(error) if view.address.is_none() => Err(error.clone()),
        _ => Ok(view),
    }
}

/// Android tap „Schließen“.
#[cfg(any(target_os = "android", all(unix, test)))]
pub(crate) fn close(device_id: &str, public_key: &str, port: u16) -> bool {
    let mut all = devices();
    let Some(device) = all.get_mut(&device_key(device_id, public_key)) else {
        return false;
    };
    let service = device.service.clone();
    device
        .mappings
        .get_mut(&port)
        .map(|mapping| mapping.stop(Some(&service)))
        .is_some()
}

fn stop_device(mut device: Device) {
    let service = device.service.clone();
    for mapping in device.mappings.values_mut() {
        mapping.stop(Some(&service));
    }
}

/// The device went offline, lost the grant or its feed ended: every
/// listener and tunnel of it ends (design 4.3).
pub(crate) fn remove(device_id: &str, public_key: &str) {
    let removed = devices().remove(&device_key(device_id, public_key));
    removed.into_iter().for_each(stop_device);
}

/// Every device's mappings end (service stop).
pub(crate) fn remove_all() {
    let removed = std::mem::take(&mut *devices());
    removed.into_values().for_each(stop_device);
}

/// The mapped services of one device, by host port.
pub(crate) fn views(device_id: &str, public_key: &str) -> Vec<MappedService> {
    let all = devices();
    let Some(device) = all.get(&device_key(device_id, public_key)) else {
        return Vec::new();
    };
    device
        .mappings
        .values()
        .filter_map(|mapping| mapping.view.lock().ok().map(|view| view.clone()))
        .collect()
}

fn start(
    service: &ShareService,
    target: &PeerOpenTarget,
    address: Ipv4Addr,
    port: u16,
    mapping: &mut Mapping,
) {
    let mut view = mapping.view.lock().unwrap_or_else(PoisonError::into_inner);
    view.last_error = None;
    let check: Option<Arc<dyn OriginCheck>> = match admission() {
        ListenerAdmission::AnyLocal => None,
        _ => match SocketOwnerCheck::new() {
            Ok(check) => Some(Arc::new(check)),
            Err(error) => {
                view.last_error = Some(error);
                return;
            }
        },
    };
    let (listener, note) = match bind_mapping(address, port) {
        Ok(bound) => bound,
        Err(error) => {
            view.last_error = Some(format!(
                "Kein lokaler Port für {port} auf {address}: {error}"
            ));
            return;
        }
    };
    let local = match listener.local_addr() {
        Ok(local) => local,
        Err(error) => {
            view.last_error = Some(error.to_string());
            return;
        }
    };
    view.address = Some(local);
    view.url = service_url(local, port);
    view.port_note = note.map(|note| format!("{port} → {local} ({note})"));
    drop(view);
    let task = serve(
        service.clone(),
        target.clone(),
        port,
        listener,
        check,
        mapping.view.clone(),
    );
    mapping.task = Some(service.screen_runtime().spawn(task));
}

async fn serve(
    service: ShareService,
    target: PeerOpenTarget,
    port: u16,
    listener: std::net::TcpListener,
    check: Option<Arc<dyn OriginCheck>>,
    view: Arc<Mutex<MappedService>>,
) {
    let note = |text: String| {
        view.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_error = Some(text);
    };
    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(listener) => listener,
        Err(error) => return note(format!("Listener nicht nutzbar: {error}")),
    };
    let local = match listener.local_addr() {
        Ok(local) => local,
        Err(error) => return note(error.to_string()),
    };
    // One sender per open tunnel; opening is serialized so two first
    // connections do not open two tunnels.
    let link: Arc<tokio::sync::Mutex<Option<mpsc::Sender<TcpStream>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // The listener ends; the next feed update starts it again.
                return note(format!(
                    "Lokaler Port {local} nimmt nichts mehr an: {error}"
                ));
            }
        };
        let service = service.clone();
        let target = target.clone();
        let check = check.clone();
        let view = view.clone();
        let link = link.clone();
        tokio::spawn(async move {
            if let Some(check) = check {
                let verdict =
                    tokio::task::spawn_blocking(move || check.same_account(peer, local)).await;
                let verdict = verdict.map_err(|error| error.to_string()).and_then(|v| v);
                if let Err(text) = verdict {
                    view.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .last_error = Some(text);
                    return;
                }
            }
            let delivered = deliver(&service, &target, port, local, socket, &link, &view).await;
            if let Err(text) = delivered {
                view.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .last_error = Some(text);
            }
        });
    }
}

/// Hands `socket` to the open tunnel or opens one. A closed tunnel returns
/// the socket, which then opens the next tunnel.
async fn deliver(
    service: &ShareService,
    target: &PeerOpenTarget,
    port: u16,
    local: SocketAddr,
    socket: TcpStream,
    link: &tokio::sync::Mutex<Option<mpsc::Sender<TcpStream>>>,
    view: &Mutex<MappedService>,
) -> Result<(), String> {
    let mut current = link.lock().await;
    let socket = match current.as_ref() {
        Some(sender) => match sender.send(socket).await {
            Ok(()) => return Ok(()),
            Err(mpsc::error::SendError(socket)) => socket,
        },
        None => socket,
    };
    let request = TcpStart {
        destination: TcpDestination::from_host_port("loopback", port)?,
        local_port: 0,
        acknowledge_local_access: true,
        lifetime_secs: 0,
    };
    let (id, sender) = service
        .tcp_open_channel(target, request, local, admission(), socket)
        .await
        .map_err(|error| error.to_string())?;
    *current = Some(sender);
    let mut shown = view.lock().unwrap_or_else(PoisonError::into_inner);
    shown.tunnel = Some(id);
    shown.last_error = None;
    Ok(())
}
