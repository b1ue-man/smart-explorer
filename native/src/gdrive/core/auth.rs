use super::api::{drive_request, err, parse_json, send_retry};
use super::core::now_secs;
use super::GDriveBackend;
use crate::cloud::{self, Provider};
use crate::vfs::VfsResult;

impl GDriveBackend {
    pub(super) fn bearer(&self) -> VfsResult<String> {
        let mut t = self.tokens_guard()?;
        if now_secs() >= t.expires_at {
            *t = cloud::refresh_access(Provider::GDrive).map_err(err)?;
        }
        Ok(t.access_token.clone())
    }

    /// Refresh even when the cached expiry has not elapsed. Drive can revoke
    /// an access token early; resumable requests retry one 401 with this path.
    pub(super) fn force_refresh_bearer(&self) -> VfsResult<String> {
        let mut tokens = self.tokens_guard()?;
        *tokens = cloud::refresh_access(Provider::GDrive).map_err(err)?;
        Ok(tokens.access_token.clone())
    }

    pub(super) fn get_json(&self, url: &str) -> VfsResult<serde_json::Value> {
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        parse_json(send_retry(|| {
            drive_request(ureq::get(url).set("Authorization", &bearer).call())
        })?)
    }
}
