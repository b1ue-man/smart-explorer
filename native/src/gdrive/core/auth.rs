use super::api::{err, parse_json, send_retry};
use super::core::now_secs;
use super::GDriveBackend;
use crate::cloud::{self, Provider};
use crate::vfs::VfsResult;

impl GDriveBackend {
    pub(super) fn bearer(&self) -> VfsResult<String> {
        let mut t = self.tokens.lock().unwrap();
        if now_secs() >= t.expires_at {
            *t = cloud::refresh_access(Provider::GDrive).map_err(err)?;
        }
        Ok(t.access_token.clone())
    }

    pub(super) fn get_json(&self, url: &str) -> VfsResult<serde_json::Value> {
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        parse_json(send_retry(|| {
            ureq::get(url).set("Authorization", &bearer).call()
        })?)
    }
}
