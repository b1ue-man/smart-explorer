//! The same key login for both signaling transports, before registration.

use std::collections::HashSet;
use std::io;

use crate::limits::validate_identifier;
use crate::login;
use crate::state::ClientIdentity;
use crate::{tracked_direct, In, Out};

pub(super) struct HelloSession {
    pub(super) device_id: String,
    pub(super) capabilities: HashSet<String>,
    pub(super) identity: ClientIdentity,
    public_key: String,
    nonce: Option<[u8; login::NONCE_BYTES]>,
}

impl HelloSession {
    pub(super) fn begin(message: In) -> io::Result<Self> {
        let In::Hello {
            protocol_version,
            device_id,
            public_key,
            capabilities,
            ..
        } = message
        else {
            return Err(rejected("first message must be hello"));
        };
        if protocol_version != 3 || validate_identifier("device id", &device_id).is_err() {
            return Err(rejected("unsupported hello"));
        }
        let capabilities = tracked_direct::negotiate_capabilities(capabilities);
        let key = login::parse_key(&public_key);
        let nonce = if capabilities.contains(login::CAPABILITY) {
            if key.is_none() {
                return Err(rejected(login::LOGIN_FAILED));
            }
            Some(login::nonce().ok_or_else(|| rejected(login::LOGIN_FAILED))?)
        } else {
            None
        };
        let identity = key.map(ClientIdentity::LegacyClaimed).unwrap_or_default();
        Ok(Self {
            device_id,
            capabilities,
            identity,
            public_key,
            nonce,
        })
    }

    pub(super) fn challenge(&self) -> Option<Out> {
        self.nonce.as_ref().map(|nonce| Out::HelloChallenge {
            nonce: login::hex(nonce),
        })
    }

    pub(super) fn answer(&mut self, message: In) -> io::Result<()> {
        let In::HelloAuth { signature } = message else {
            return Err(rejected(login::LOGIN_FAILED));
        };
        let nonce = self
            .nonce
            .take()
            .ok_or_else(|| rejected(login::LOGIN_FAILED))?;
        let key = login::verify(&nonce, &self.device_id, &self.public_key, &signature)
            .ok_or_else(|| rejected(login::LOGIN_FAILED))?;
        self.identity = ClientIdentity::Proven(key);
        Ok(())
    }
}

fn rejected(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
