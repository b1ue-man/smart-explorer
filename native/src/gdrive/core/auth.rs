use super::api::{drive_request, open_stream, parse_json, send_retry};
use super::core::now_secs;
use super::GDriveBackend;
use crate::cloud::{self, Provider};
use crate::vfs::VfsResult;
use std::io;

impl GDriveBackend {
    /// The current access token. An expired token is refreshed while the
    /// token lock is held, so concurrent callers wait for that one refresh
    /// instead of starting their own.
    pub(super) fn bearer(&self) -> VfsResult<String> {
        if self.managed_auth {
            cloud::validate_auth_account(Provider::GDrive, &self.drive_account_key).map_err(auth_error)?;
        }
        let mut t = self.tokens_guard()?;
        if now_secs() >= t.expires_at {
            *t = self.refresh_for_account()?;
        }
        Ok(t.access_token.clone())
    }

    /// Refresh even when the cached expiry has not elapsed. Drive can revoke
    /// an access token early; resumable requests retry one 401 with this path.
    pub(super) fn force_refresh_bearer(&self) -> VfsResult<String> {
        let mut tokens = self.tokens_guard()?;
        *tokens = self.refresh_for_account()?;
        Ok(tokens.access_token.clone())
    }

    /// A concurrent read may already have refreshed the rejected token.
    /// Never turn a wave of old-token 401s into repeated OAuth refreshes.
    fn refresh_rejected_bearer(&self, rejected: &str) -> VfsResult<String> {
        let mut tokens = self.tokens_guard()?;
        if tokens.access_token == rejected {
            *tokens = self.refresh_for_account()?;
        }
        Ok(tokens.access_token.clone())
    }

    /// A stored OAuth credential may have changed accounts since this backend
    /// was connected. Validate the principal before adopting its new token.
    /// This uses the explicit token and never re-enters the held token lock.
    fn refresh_for_account(&self) -> VfsResult<cloud::Tokens> {
        cloud::refresh_access_with_identity(
            Provider::GDrive, Some(self.drive_account_key.as_ref()), |token| {
                let body = self.metadata_text(&self.api_url("about?fields=user(permissionId)"), token)
                    .map_err(|error| match super::overload::http_status(&error) {
                        Some(status) => cloud::AuthError::provider(status, "provider_error"),
                        None => cloud::AuthError::new(cloud::AuthState::NetworkError),
                    })?;
                super::state::parse_drive_account_key(&body)
                    .map_err(|_| cloud::AuthError::new(cloud::AuthState::IdentityUnverified))
            },
        ).map_err(auth_error)
    }

    /// Metadata GET over the pooled API client, read completely so the socket
    /// returns to the pool.
    pub(super) fn get_json(&self, url: &str) -> VfsResult<serde_json::Value> {
        let mut token = self.bearer()?;
        for attempt in 0..2 {
            match self.metadata_text(url, &token) {
                Ok(body) => return parse_json(body),
                Err(error) if attempt == 0 && super::overload::http_status(&error) == Some(401) => {
                    token = self.refresh_rejected_bearer(&token)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other(
            "Drive metadata authentication retry did not finish",
        ))
    }

    fn metadata_text(&self, url: &str, token: &str) -> VfsResult<String> {
        let bearer = format!("Bearer {token}");
        let agent = self.http.api();
        send_retry(|| {
            drive_request(
                self.timed_request(agent.get(url))
                    .set("Authorization", &bearer)
                    .call(),
            )
        })
    }

    /// Retry one rejected access token before exposing a streaming body.
    /// Once bytes escape, normal transfer restart/cancellation owns recovery.
    pub(super) fn authenticated_stream(
        &self,
        url: &str,
        range: Option<&str>,
    ) -> VfsResult<ureq::Response> {
        let mut token = self.bearer()?;
        let agent = self.http.stream();
        for attempt in 0..2 {
            let bearer = format!("Bearer {token}");
            let response = open_stream(|| {
                let mut request = agent.get(url).set("Authorization", &bearer);
                if let Some(range) = range {
                    request = request.set("Range", range);
                }
                drive_request(request.call())
            });
            match response {
                Ok(response) => return Ok(response),
                Err(error) if attempt == 0 && super::overload::http_status(&error) == Some(401) => {
                    token = self.refresh_rejected_bearer(&token)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other(
            "Drive stream authentication retry did not finish",
        ))
    }
}

fn auth_error(error: cloud::AuthError) -> io::Error {
    let kind = match error.state {
        cloud::AuthState::IdentityConflict | cloud::AuthState::ClientChanged |
        cloud::AuthState::ReauthorizeRequired | cloud::AuthState::Disconnected => io::ErrorKind::PermissionDenied,
        cloud::AuthState::NetworkError => io::ErrorKind::ConnectionAborted,
        cloud::AuthState::IdentityUnverified => io::ErrorKind::InvalidData,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}
