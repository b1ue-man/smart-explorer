//! A live IPC connection owns exactly the exchange it successfully starts.
//! Event observers are independent of the GUI's consumable event queue.
use std::io::{self, Read};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::share::{DiscoveryCommand, DiscoveryEvent, ShareCmd, ShareCmdResult, ShareService};
use super::discovery_pairing_protocol::{
    DiscoveryPairingError as Error, DiscoveryPairingReply as Reply,
    DiscoveryPairingRequest as Request, DiscoveryPairingStatus as Status,
};
use super::ipc_host::ShareHost;
use super::ipc_protocol::{write_response, IpcResponse};

const POLL: Duration = Duration::from_millis(100);
const LIST_WAIT: Duration = Duration::from_secs(15);
// The core exchange deadline is two minutes. Allow the maintenance event to
// reach the host before giving up observation; this is not a pairing retry.
const EXCHANGE_WAIT: Duration = Duration::from_secs(130);
// There are at most eight simultaneous core exchanges. Before the start
// reply identifies ours, keep at most that many terminal events. An overflow
// fails closed instead of dropping a possibly relevant result silently.
const PENDING_TERMINALS: usize = 8;

#[derive(Default)]
pub(super) struct DiscoveryObservers {
    observers: Vec<Weak<Mutex<Observation>>>,
}

impl DiscoveryObservers {
    fn subscribe(&mut self, discovery_id: Option<String>) -> Arc<Mutex<Observation>> {
        self.observers.retain(|observer| observer.strong_count() > 0);
        let observation = Arc::new(Mutex::new(Observation {
            discovery_id,
            ..Observation::default()
        }));
        self.observers.push(Arc::downgrade(&observation));
        observation
    }

    pub(super) fn observe(&mut self, event: &DiscoveryEvent) {
        self.observers.retain(|observer| {
            let Some(observer) = observer.upgrade() else { return false; };
            if let Ok(mut state) = observer.lock() { state.observe(event); }
            true
        });
    }
}

#[derive(Default)]
struct Observation {
    discovery_id: Option<String>,
    exchange_id: Option<String>,
    list: Option<Vec<crate::share::DiscoveryAdvertisement>>,
    terminal: Vec<DiscoveryEvent>,
    overflow: bool,
}

impl Observation {
    fn observe(&mut self, event: &DiscoveryEvent) {
        if let DiscoveryEvent::DiscoveryList { advertisements } = event {
            self.list = Some(advertisements.clone());
            return;
        }
        let Some(wanted_discovery) = self.discovery_id.as_deref() else { return; };
        let (exchange_id, discovery_id) = match event {
            DiscoveryEvent::ExchangeCompleted { exchange_id, discovery_id, .. } =>
                (Some(exchange_id.as_str()), Some(discovery_id.as_str())),
            DiscoveryEvent::ExchangeUnconfirmed { exchange_id, discovery_id, .. }
            | DiscoveryEvent::ExchangeCancelled { exchange_id, discovery_id } =>
                (Some(exchange_id.as_str()), discovery_id.as_deref()),
            DiscoveryEvent::ExchangeFailed { exchange_id, discovery_id, .. } =>
                (exchange_id.as_deref(), discovery_id.as_deref()),
            _ => return,
        };
        let relevant = match self.exchange_id.as_deref() {
            Some(ours) => exchange_id == Some(ours),
            None => discovery_id.is_none() || discovery_id == Some(wanted_discovery),
        };
        if !relevant { return; }
        if self.terminal.len() >= PENDING_TERMINALS {
            self.overflow = true;
        } else {
            self.terminal.push(event.clone());
        }
    }

    fn take_result(&mut self, exchange_id: &str, discovery_id: &str) -> Option<Reply> {
        self.exchange_id = Some(exchange_id.to_string());
        for event in self.terminal.drain(..) {
            let (id, status, outcome, error_code) = match event {
                DiscoveryEvent::ExchangeCompleted { exchange_id, outcome, .. } =>
                    (Some(exchange_id), Status::Completed, Some(outcome), None),
                DiscoveryEvent::ExchangeUnconfirmed { exchange_id, outcome, .. } =>
                    (Some(exchange_id), Status::Unconfirmed, Some(outcome), None),
                DiscoveryEvent::ExchangeCancelled { exchange_id, .. } =>
                    (Some(exchange_id), Status::Cancelled, None, Some(Error::Cancelled)),
                DiscoveryEvent::ExchangeFailed { exchange_id, .. } =>
                    (exchange_id, Status::Failed, None, Some(Error::PairingFailed)),
                _ => continue,
            };
            if id.as_deref() == Some(exchange_id) {
                return Some(Reply::Finished {
                    discovery_id: discovery_id.to_string(),
                    exchange_id: exchange_id.to_string(),
                    status,
                    outcome,
                    error_code,
                });
            }
        }
        None
    }
}

struct OwnedExchange {
    service: ShareService,
    exchange_id: Option<String>,
    connection: ConnectionWatch,
    // Declared last: the immediate Cancel can still use the original
    // service; then dropping this handle lets maintenance close any start
    // whose acknowledgement was lost. The worker retains its own lease.
    _owner: crate::share::DiscoveryExchangeOwner,
}

impl Drop for OwnedExchange {
    fn drop(&mut self) {
        self.connection.cancel.cancel();
        if let Some(exchange_id) = self.exchange_id.take() {
            // Use the original service: a replacement worker must never
            // acquire or cancel an exchange belonging to this connection.
            let _ = self.service.cmd(ShareCmd::Discovery(
                DiscoveryCommand::CancelDiscoveryExchange { exchange_id },
            ));
        }
    }
}

/// Read the owner connection even while the worker is busy acknowledging a
/// command. A hard child exit must cancel before a late acknowledgement.
struct ConnectionWatch {
    cancel: crate::share::DiscoveryExchangeCancellation,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ConnectionWatch {
    fn new(stream: &TcpStream, cancel: crate::share::DiscoveryExchangeCancellation) -> io::Result<Self> {
        let mut read = stream.try_clone()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let cancelled = cancel.clone();
        let thread = std::thread::Builder::new().name("discovery-owner".into()).spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match connection_alive(&mut read) {
                    Ok(true) => {}
                    // EOF, explicit cancel, malformed control and socket
                    // errors all end only this connection's ownership.
                    Ok(false) | Err(_) => { cancelled.cancel(); break; }
                }
            }
        })?;
        Ok(Self { cancel, stop, thread: Some(thread) })
    }
}

impl Drop for ConnectionWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

pub(super) fn serve(
    mut stream: TcpStream,
    host: &ShareHost,
    request: Request,
    runtime_lease: Option<Arc<dyn Send + Sync>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(POLL))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    if host.reload_now().is_err() {
        return answer(&mut stream, Reply::Error { error_code: Error::DaemonError });
    }
    host.drain_events();
    let discovery_id = match &request {
        Request::List => None,
        Request::Pair { discovery_id, pin, .. } => {
            if pin.as_bytes().is_empty() || pin.as_bytes().len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
                return answer(&mut stream, Reply::Error { error_code: Error::InvalidPin });
            }
            Some(discovery_id.clone())
        }
    };
    let (service, observation) = {
        let mut state = host.state.lock().map_err(|_| io::Error::other("discovery state unavailable"))?;
        let Some(service) = state.service.clone() else {
            return answer(&mut stream, Reply::Error { error_code: Error::WorkerUnavailable });
        };
        if !state.signal_connected {
            return answer(&mut stream, Reply::Error { error_code: Error::Offline });
        }
        (service, state.discovery_pairing.subscribe(discovery_id))
    };
    if let Err(error) = service.cmd(ShareCmd::Discovery(DiscoveryCommand::ListDiscoveries)) {
        return answer(&mut stream, Reply::Error { error_code: Error::from_command(&error) });
    }
    let offers = match wait_list(&mut stream, host, &observation) {
        Ok(offers) => offers,
        Err(error_code) => return answer(&mut stream, Reply::Error { error_code }),
    };
    match request {
        Request::List => answer(&mut stream, Reply::List { offers }),
        Request::Pair { discovery_id, pin, share_back } => {
            if !offers.iter().any(|offer| offer.discovery_id == discovery_id
                && offer.expires_at > crate::share::core_now_secs())
            {
                return answer(&mut stream, Reply::Error { error_code: Error::OfferUnavailable });
            }
            let (owner_handle, lease) = crate::share::DiscoveryExchangeOwner::new(runtime_lease);
            let connection = ConnectionWatch::new(&stream, owner_handle.cancellation())?;
            // Create the cancellation handle before submitting the command.
            // An ACK timeout must not orphan the worker's eventual start.
            let mut owner = OwnedExchange {
                service: service.clone(), exchange_id: None, connection, _owner: owner_handle,
            };
            let started = service.cmd(ShareCmd::Discovery(DiscoveryCommand::StartOwnedDiscoveryExchange {
                discovery_id: discovery_id.clone(), pin, share_back, owner: lease,
            }));
            let exchange_id = match started {
                Ok(ShareCmdResult::DiscoveryExchange(handle)) => handle.exchange_id,
                Err(error) => {
                    owner.connection.cancel.cancel();
                    return answer(&mut stream, Reply::Error { error_code: Error::from_command(&error) });
                }
                _ => return answer(&mut stream, Reply::Error { error_code: Error::DaemonError }),
            };
            owner.exchange_id = Some(exchange_id.clone());
            answer(&mut stream, Reply::Started { discovery_id: discovery_id.clone(), exchange_id: exchange_id.clone() })?;
            let deadline = Instant::now() + EXCHANGE_WAIT;
            loop {
                host.drain_events();
                let result = {
                    let mut state = observation.lock().map_err(|_| io::Error::other("discovery observation unavailable"))?;
                    if state.overflow { return answer(&mut stream, Reply::Error { error_code: Error::ObservationLost }); }
                    state.take_result(&exchange_id, &discovery_id)
                };
                if let Some(result) = result {
                    owner.exchange_id = None;
                    return answer(&mut stream, result);
                }
                if Instant::now() >= deadline {
                    return answer(&mut stream, Reply::Error { error_code: Error::Timeout });
                }
                if owner.connection.cancel.is_cancelled() { return Ok(()); }
                std::thread::sleep(POLL);
            }
        }
    }
}

fn wait_list(
    stream: &mut TcpStream,
    host: &ShareHost,
    observation: &Mutex<Observation>,
) -> Result<Vec<crate::share::DiscoveryAdvertisement>, Error> {
    let deadline = Instant::now() + LIST_WAIT;
    loop {
        host.drain_events();
        if let Some(list) = observation.lock().map_err(|_| Error::ObservationLost)?.list.take() {
            return Ok(list);
        }
        if Instant::now() >= deadline { return Err(Error::Timeout); }
        if !connection_alive(stream).map_err(|_| Error::DaemonError)? { return Err(Error::Cancelled); }
    }
}

fn connection_alive(stream: &mut TcpStream) -> io::Result<bool> {
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        // EOF covers hard process termination on Windows as well as drop.
        Ok(0) => Ok(false),
        Ok(1) if byte[0] == b'c' => Ok(false),
        Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid discovery control")),
        Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => Ok(true),
        Err(error) => Err(error),
    }
}

fn answer(stream: &mut TcpStream, reply: Reply) -> io::Result<()> {
    write_response(stream, &IpcResponse::DiscoveryPairing { reply })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed(id: &str) -> DiscoveryEvent {
        DiscoveryEvent::ExchangeCompleted {
            exchange_id: id.into(),
            discovery_id: "offer".into(),
            outcome: crate::share::DiscoveryRelationOutcome::DirectInstalled {
                contact_id: "contact".into(), display_name: "Peer".into(),
            },
        }
    }

    #[test]
    fn batches_task_p29_gui_drain_preserves_exact_owned_terminal_result() {
        let mut host = super::super::ipc_host::ShareHostState::new();
        let observer = host.discovery_pairing.subscribe(Some("offer".into()));
        host.discovery_pairing.observe(&completed("previous-exchange"));
        host.discovery_pairing.observe(&completed("ours"));
        host.ui_events.push(crate::share::ShareEvent::Discovery(completed("ours")));
        let _gui = std::mem::take(&mut host.ui_events);
        let result = observer.lock().unwrap().take_result("ours", "offer").unwrap();
        assert!(matches!(result, Reply::Finished {
            exchange_id, status: Status::Completed, ..
        } if exchange_id == "ours"));
        assert!(host.ui_events.is_empty());
    }

    #[test]
    fn batches_task_p29_observer_overflow_fails_closed_and_dead_observers_expire() {
        let mut observers = DiscoveryObservers::default();
        let observer = observers.subscribe(Some("offer".into()));
        for index in 0..PENDING_TERMINALS + 1 {
            observers.observe(&completed(&index.to_string()));
        }
        let state = observer.lock().unwrap();
        assert!(state.overflow);
        assert_eq!(state.terminal.len(), PENDING_TERMINALS);
        drop(state);
        drop(observer);
        observers.observe(&completed("irrelevant"));
        assert!(observers.observers.is_empty());
    }

    #[test]
    fn batches_task_p29_connection_end_cancels_even_before_start_acknowledgement() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        server.set_read_timeout(Some(POLL)).unwrap();
        let (owner, _pending) = crate::share::DiscoveryExchangeOwner::new(None);
        let watched = ConnectionWatch::new(&server, owner.cancellation()).unwrap();
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !watched.cancel.is_cancelled() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(watched.cancel.is_cancelled());
    }
}
