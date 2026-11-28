//! Opening an `AgentBackend`: the handshake over a framed stream pair and the
//! connection pool it starts with (moved out of `backend.rs` unchanged).
use super::super::pool::AgentPool;
#[cfg(any(test, feature = "test-support"))]
use super::super::transport::HeartbeatPolicy;
use super::super::transport::{AgentConnection, AgentReconnect};
use super::AgentBackend;
use crate::agent_proto::ServerFeatures;
use crate::vfs::BackendHandle;
use std::io::{self, Read, Write};

impl AgentBackend {
    /// Hand-shake over an already-open framed stream pair.
    pub fn from_streams(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
    ) -> io::Result<Self> {
        Self::from_streams_inner((r, w), inner, None, None)
    }

    pub(in super::super) fn from_streams_with_reconnect(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        reconnect: AgentReconnect,
    ) -> io::Result<Self> {
        Self::from_streams_inner((r, w), inner, Some(reconnect), None)
    }

    /// The SSH-deployed agent: its reconnect also opens further exec
    /// channels, so this backend may spread requests over a channel pool.
    pub(in super::super) fn from_root_confined_streams_with_reconnect(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        reconnect: AgentReconnect,
        root: String,
    ) -> io::Result<Self> {
        Self::from_streams_inner((r, w), inner, Some(reconnect), Some(root))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in super::super) fn from_streams_with_reconnect_and_heartbeat(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        reconnect: AgentReconnect,
        heartbeat: HeartbeatPolicy,
    ) -> io::Result<Self> {
        let (connection, version) =
            AgentConnection::new_with_heartbeat((r, w), Some(reconnect), heartbeat)?;
        let pool = AgentPool::single(connection, ServerFeatures::parse(&version));
        Ok(Self {
            inner,
            pool,
            version,
            root_confined: None,
        })
    }

    /// A pool over test streams: `opener` provides further channels.
    #[cfg(any(test, feature = "test-support"))]
    pub(in super::super) fn pooled_for_test(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        opener: AgentReconnect,
    ) -> io::Result<Self> {
        let (connection, version) = AgentConnection::new((r, w), Some(opener.clone()))?;
        let pool = AgentPool::growable(connection, ServerFeatures::parse(&version), opener);
        Ok(Self {
            inner,
            pool,
            version,
            root_confined: None,
        })
    }

    fn from_streams_inner(
        streams: super::super::transport::AgentStreams,
        inner: BackendHandle,
        reconnect: Option<AgentReconnect>,
        root_confined: Option<String>,
    ) -> io::Result<Self> {
        let (connection, version) = AgentConnection::new(streams, reconnect.clone())?;
        let features = ServerFeatures::parse(&version);
        let pool = match (reconnect, root_confined.is_some()) {
            (Some(opener), true) => AgentPool::growable(connection, features, opener),
            _ => AgentPool::single(connection, features),
        };
        Ok(AgentBackend {
            inner,
            pool,
            version,
            root_confined,
        })
    }
}
