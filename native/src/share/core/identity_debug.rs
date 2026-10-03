//! Identity diagnostics deliberately omit private keys and usable pairing codes.
use std::fmt;

use super::identity::{DirectCodeRotation, ShareIdentity};

impl fmt::Debug for ShareIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShareIdentity")
            .field("device_id", &self.device_id)
            .field("device_name", &self.device_name)
            .field("public_key", &self.public_key)
            .field("fingerprint", &self.fingerprint)
            .field("node_id", &self.node_id)
            .field("direct_lookup_id", &"[redacted]")
            .field("iroh_secret", &"[redacted]")
            .field("direct_secret", &"[redacted]")
            .finish()
    }
}

impl fmt::Debug for DirectCodeRotation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectCodeRotation")
            .field("code", &"[redacted]")
            .field("cleanup_warning", &self.cleanup_warning)
            .finish()
    }
}
