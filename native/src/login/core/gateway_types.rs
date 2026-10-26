//! Separate machine endpoint and bounded remote peer contracts; no user profile.
use std::{io, net::SocketAddr};
use serde::{Deserialize, Serialize};
use super::{model::{Challenge, LoginRights, Policy, SignedRequest, VERSION, MAX_PIXELS}, wire::Reply};
pub(crate) const LOGIN_ALPN: &[u8] = b"smart-explorer/login/1";
pub(crate) const NETWORK_FILE: &str = "gateway-network.json";
pub(crate) const ADDRESS_FILE: &str = "gateway-address.json";
pub(crate) const MACHINE_FILE: &str = "machine-key.json";
pub(crate) const MAX_NETWORK: usize = 32 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct GatewayNetwork {
    pub format: u32,
    pub use_default_relays: bool,
    pub relay_urls: Vec<String>,
    pub relay_pins: Vec<[u8; 32]>,
    pub allow_plaintext_relays: bool,
}
impl Default for GatewayNetwork {
    fn default() -> Self { Self { format: VERSION, use_default_relays: true, relay_urls: Vec::new(), relay_pins: Vec::new(), allow_plaintext_relays: false } }
}
impl GatewayNetwork {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.format != VERSION || self.relay_urls.len() > 16 || self.relay_pins.len() > 16
            || self.relay_urls.iter().any(|url| url.len() > 2048 || url.contains(['\0','\r','\n']))
            || (self.use_default_relays && !self.relay_urls.is_empty()) {
            return Err(invalid("invalid explicit login gateway network options"));
        }
        if serde_json::to_vec(self).map_err(invalid)?.len() > MAX_NETWORK { return Err(invalid("gateway network record exceeds its byte budget")); }
        for url in &self.relay_urls {
            let parsed: iroh::RelayUrl = url.parse().map_err(invalid)?;
            if !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() || parsed.fragment().is_some() {
                return Err(invalid("login relay URLs cannot embed credentials, queries or fragments"));
            }
            if parsed.scheme() != "https" && !(self.allow_plaintext_relays && parsed.scheme() == "http") {
                return Err(invalid("plaintext login relays require explicit permission"));
            }
        }
        Ok(())
    }
}

/// Public pin plus routing hints. Addresses never provide peer authorization.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct GatewayTarget {
    pub format: u32,
    pub machine_id: String,
    pub public_key: String,
    pub direct_addresses: Vec<SocketAddr>,
    pub relay_urls: Vec<String>,
    pub use_default_relays: bool,
    pub allow_plaintext_relays: bool,
}
impl GatewayTarget {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.format != VERSION || self.machine_id.is_empty() || self.machine_id.len() > 256
            || self.public_key.parse::<iroh::PublicKey>().is_err() || self.direct_addresses.len() > 32
            || self.direct_addresses.iter().any(|addr| addr.port() == 0 || addr.ip().is_unspecified() || addr.ip().is_multicast()) {
            return Err(invalid("invalid pinned login machine target"));
        }
        GatewayNetwork { format: VERSION, use_default_relays: self.use_default_relays,
            relay_urls: self.relay_urls.clone(), relay_pins: Vec::new(), allow_plaintext_relays: self.allow_plaintext_relays }.validate()?;
        if serde_json::to_vec(self).map_err(invalid)?.len() > MAX_NETWORK { return Err(invalid("gateway target record exceeds its byte budget")); }
        Ok(())
    }
    pub(crate) fn check_challenge(&self, challenge: &Challenge, user: &str) -> io::Result<()> {
        if challenge.format != VERSION || challenge.machine_id != self.machine_id || challenge.machine_public_key != self.public_key
            || challenge.binding.user_id.as_deref().is_some_and(|actual| actual != user)
            || challenge.sequence == 0 || challenge.nonce.len() != 64 || !challenge.nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
            || challenge.boot_generation.is_empty() || challenge.boot_generation.len() > 256
            || challenge.binding.session_id.is_empty() || challenge.binding.session_id.len() > 256
            || challenge.binding.session_generation.is_empty() || challenge.binding.session_generation.len() > 1024
            || challenge.binding.desktop_generation.is_empty() || challenge.binding.desktop_generation.len() > 2048
            || challenge.policy_revision == 0 {
            return Err(invalid("broker challenge does not match the pinned machine or target user"));
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerHello { pub format: u32, pub machine_id: String, pub public_key: String, pub route_policy: u32, pub admission: String }
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientHello { pub format: u32, pub peer_id: String, pub target_user: String, pub route_policy: u32 }
impl ClientHello {
    pub(crate) fn authorize<'a>(&self, policy: &'a Policy, remote: &str, now: u64) -> io::Result<&'a super::model::PeerGrant> {
        if self.format != VERSION || self.route_policy != VERSION || self.peer_id.is_empty() || self.peer_id.len() > 256
            || self.target_user.is_empty() || self.target_user.len() > 256 { return Err(invalid("invalid login client hello")); }
        let grant = policy.grants.iter().find(|grant| grant.peer_id == self.peer_id && grant.public_key == remote)
            .ok_or_else(|| denied("authenticated transport peer has no matching administrative login grant"))?;
        if !grant.target_users.contains(&self.target_user) || now < grant.not_before || now >= grant.expires_at
            || !grant.rights.login || !grant.rights.screen_view { return Err(denied("login peer, target user, lifetime or rights are not authorized")); }
        Ok(grant)
    }
    pub(crate) fn check_operation(&self, request: &SignedRequest, public_key: &str, challenge: &Challenge) -> io::Result<()> {
        if request.peer_id != self.peer_id || request.target_user != self.target_user || &request.challenge != challenge {
            return Err(denied("signed login request differs from its authenticated transport session"));
        }
        request.operation.validate().map_err(invalid)?;
        if !crate::share::verify_iroh_signature(public_key, &request.signing_bytes()?, &request.signature) {
            return Err(denied("login operation signature does not match the authenticated remote peer"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GatewayAccess {
    pub peer_id: String, pub public_key: String, pub target_user: String,
    pub policy_revision: u64, pub not_before: u64, pub expires_at: u64, pub rights: LoginRights,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GatewayReply {
    pub request_sequence: u64, pub reply: Reply,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<GatewayAccess>,
}
impl GatewayReply {
    pub(crate) fn validate_pixels(&self, bytes: usize) -> io::Result<()> {
        let expected = (self.reply.width as usize).checked_mul(self.reply.height as usize).and_then(|size| size.checked_mul(4));
        if bytes != self.reply.pixel_bytes || bytes > MAX_PIXELS
            || (bytes != 0 && self.reply.state != "pixels") || (bytes == 0 && self.reply.state == "pixels")
            || (bytes != 0 && (self.reply.width == 0 || self.reply.height == 0 || expected != Some(bytes))) {
            return Err(invalid("login pixel frame exceeds its declared dimensions or byte limit"));
        }
        Ok(())
    }
}
fn invalid(error: impl ToString) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, error.to_string()) }
fn denied(reason: &str) -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, reason) }
