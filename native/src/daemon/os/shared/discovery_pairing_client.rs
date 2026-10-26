//! The socket remains open until the caller observes a terminal pairing state.
use std::io::Write;
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use super::discovery_pairing_protocol::{
    DiscoveryPairingError as Error, DiscoveryPairingReply as Reply,
    DiscoveryPairingRequest as Request,
};
use super::ipc_protocol::{read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse};

fn connect(request: Request) -> Result<TcpStream, Error> {
    super::ipc_client::ensure_worker_ready().map_err(|_| Error::WorkerUnavailable)?;
    let token = super::ipc_storage::read_token().map_err(|_| Error::DaemonError)?;
    let address = super::ipc_storage::read_ipc_addr().ok_or(Error::WorkerUnavailable)?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|_| Error::WorkerUnavailable)?;
    set_stream_timeout(&stream, Some(Duration::from_secs(150)));
    write_request(&mut stream, &IpcRequest::DiscoveryPairing { token, request })
        .map_err(|_| Error::DaemonError)?;
    Ok(stream)
}

fn receive(stream: &mut TcpStream) -> Result<Reply, Error> {
    match read_response(stream).map_err(|_| Error::DaemonError)? {
        IpcResponse::DiscoveryPairing { reply: Reply::Error { error_code } } => Err(error_code),
        IpcResponse::DiscoveryPairing { reply } => Ok(reply),
        _ => Err(Error::DaemonError),
    }
}

pub(crate) fn list_discoveries() -> Result<Vec<crate::share::DiscoveryAdvertisement>, Error> {
    let mut stream = connect(Request::List)?;
    match receive(&mut stream)? {
        Reply::List { offers } => Ok(offers),
        _ => Err(Error::DaemonError),
    }
}

pub(crate) struct DiscoveryPairingSession {
    stream: TcpStream,
    discovery_id: String,
    exchange_id: String,
}

impl DiscoveryPairingSession {
    pub(crate) fn exchange_id(&self) -> &str { &self.exchange_id }

    pub(crate) fn wait(mut self) -> Result<Reply, Error> {
        match receive(&mut self.stream)? {
            reply @ Reply::Finished { .. } => {
                if let Reply::Finished { discovery_id, exchange_id, .. } = &reply {
                    if discovery_id != &self.discovery_id || exchange_id != &self.exchange_id {
                        return Err(Error::DaemonError);
                    }
                }
                Ok(reply)
            }
            _ => Err(Error::DaemonError),
        }
    }

    /// An explicit cancel closes the same owner connection; it never names
    /// or adopts an exchange started by a different GUI, CLI or MCP job.
    #[allow(dead_code)]
    pub(crate) fn cancel(mut self) -> Result<(), Error> {
        let sent = self.stream.write_all(b"c").map_err(|_| Error::DaemonError);
        let closed = self.stream.shutdown(Shutdown::Both).map_err(|_| Error::DaemonError);
        sent.and(closed)
    }
}

pub(crate) fn start_discovery_pairing(
    discovery_id: String,
    pin: crate::share::DiscoveryPin,
    share_back: bool,
) -> Result<DiscoveryPairingSession, Error> {
    if pin.as_bytes().is_empty() || pin.as_bytes().len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
        return Err(Error::InvalidPin);
    }
    let mut stream = connect(Request::Pair { discovery_id: discovery_id.clone(), pin, share_back })?;
    match receive(&mut stream)? {
        Reply::Started { discovery_id: received, exchange_id } if received == discovery_id =>
            Ok(DiscoveryPairingSession { stream, discovery_id, exchange_id }),
        _ => Err(Error::DaemonError),
    }
}
