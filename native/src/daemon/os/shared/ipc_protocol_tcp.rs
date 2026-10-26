//! Authenticated daemon control for one explicitly acknowledged TCP service.
//! Raw payload never passes through daemon JSON or MCP tool results.
use crate::share::tcp_forward::{TcpStart, TunnelId, TunnelView};
use crate::share::PeerOpenTarget;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TcpIpcRequest {
    Start {
        target: PeerOpenTarget,
        request: TcpStart,
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
    Listed { tunnels: Vec<TunnelView> },
    Status { tunnel: Option<TunnelView> },
    Closed { closed: bool },
}
