//! Negotiated P14 client calls. Mutations never replay an ambiguous request;
//! identity is read-only metadata and may reconnect under its own deadline.
use super::super::{
    agent_error::agent_error, backend::AgentBackend, ext_stream::unexpected, ext_wire::unsupported,
};
use crate::agent_proto::Frame;
use crate::vfs::Backend;
use std::{io, time::Duration};

const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) fn supported(backend: &AgentBackend, root: &str) -> bool {
    if !backend.features().folder_reservation {
        return false;
    }
    matches!(
        backend.pool.lease().safe_call_timeout(
            Frame::DirectoryReservationSupported(root.into()),
            QUERY_TIMEOUT,
        ),
        Ok(Frame::Answer(1))
    )
}

pub(super) fn create(backend: &AgentBackend, path: &str) -> io::Result<()> {
    if !backend.features().folder_reservation {
        return Err(unsupported());
    }
    let lease = backend.pool.lease();
    let (mux, reply) = lease.mutation_call(Frame::CreateDirExclusive(path.into()))?;
    match reply {
        Frame::Ok => Ok(()),
        Frame::Err(message) => Err(error(message)),
        frame => {
            lease.invalidate(&mux);
            Err(unexpected("exclusive directory", &frame))
        }
    }
}

pub(super) fn identity(backend: &AgentBackend, path: &str) -> io::Result<Option<String>> {
    if !backend.features().folder_reservation {
        return Err(unsupported());
    }
    let reply = backend
        .pool
        .lease()
        .safe_call_timeout(Frame::DirectoryIdentity(path.into()), QUERY_TIMEOUT)?;
    match reply {
        Frame::DirectoryIdentityValue(identity)
            if identity.as_ref().is_none_or(|id| !id.is_empty()) =>
        {
            Ok(identity)
        }
        Frame::Err(message) => Err(error(message)),
        frame => Err(unexpected("directory identity", &frame)),
    }
}

fn error(message: String) -> io::Error {
    Frame::folder_error(&message).unwrap_or_else(|| agent_error(message))
}

/// Only the negotiated host can attest a file object. In particular, a
/// daemon serving an old peer must not substitute its local metadata.
pub(super) fn file_identity(backend: &AgentBackend, path: &str) -> io::Result<Option<String>> {
    if !backend.features().file_identity {
        return Ok(None);
    }
    let reply = backend
        .pool
        .lease()
        .safe_call_timeout(Frame::FileIdentity(path.into()), QUERY_TIMEOUT)?;
    match reply {
        Frame::FileIdentityValue(identity) => {
            let namespace = backend.namespace_identity();
            Ok(identity.filter(|id| !id.is_empty()).map(|id| {
                format!(
                    "agent-file:{}:{namespace}:{}:{id}",
                    namespace.len(),
                    id.len()
                )
            }))
        }
        Frame::Err(message) => Err(error(message)),
        frame => Err(unexpected("file identity", &frame)),
    }
}
