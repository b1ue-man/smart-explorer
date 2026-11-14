//! Authenticated daemon control for one TCP service (design 9.3). Raw
//! payload never passes through daemon JSON or MCP tool results.
use crate::share::auto_forward::MappedService;
use crate::share::tcp_forward::{ListenerAdmission, TcpStart, TunnelId, TunnelView};
use crate::share::PeerOpenTarget;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TcpIpcRequest {
    Start {
        target: PeerOpenTarget,
        request: TcpStart,
        /// Which local connections the listener forwards; `SameAccount`
        /// unless the caller names another rule.
        #[serde(default)]
        admission: ListenerAdmission,
        /// The device address (design 4.3) instead of 127.0.0.1.
        #[serde(default)]
        bind: Option<Ipv4Addr>,
    },
    /// Only on the stream of an `AwaitProcess` start, after `Opened`: the one
    /// client process whose connections the listener forwards.
    BindProcess {
        id: TunnelId,
        pid: u32,
        started: u64,
    },
    List,
    Status {
        id: TunnelId,
    },
    Close {
        id: TunnelId,
    },
    /// Desktop „Öffnen“ (E27): opens the listener of a service that the
    /// device's current feed names, also above the automatic budget.
    OpenService {
        device_id: String,
        public_key: String,
        port: u16,
    },
    /// Desktop „Schließen“ (E27): ends the service; it stays closed until
    /// `OpenService`.
    CloseService {
        device_id: String,
        public_key: String,
        port: u16,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TcpIpcReply {
    Pending,
    Opened { tunnel: TunnelView },
    Bound { tunnel: TunnelView },
    Listed { tunnels: Vec<TunnelView> },
    Status { tunnel: Option<TunnelView> },
    Closed { closed: bool },
    // The service after `OpenService`, with its address here if it listens.
    Service { mapping: MappedService },
}
