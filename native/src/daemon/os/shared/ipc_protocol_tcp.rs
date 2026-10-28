//! Authenticated daemon control for one TCP service (design 9.3). Raw
//! payload never passes through daemon JSON or MCP tool results.
use crate::share::tcp_forward::{ListenerAdmission, TcpStart, TunnelId, TunnelView};
use crate::share::PeerOpenTarget;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TcpIpcRequest {
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
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TcpIpcReply {
    Pending,
    Opened { tunnel: TunnelView },
    Bound { tunnel: TunnelView },
    Listed { tunnels: Vec<TunnelView> },
    Status { tunnel: Option<TunnelView> },
    Closed { closed: bool },
}
