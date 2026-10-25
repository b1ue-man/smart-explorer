//! Bounded token/identity requests; provider bodies and descriptions never escape.
use crate::cloud::{AuthError, AuthState, ClientConfig, Provider, Tokens};
use std::io::Read;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RESPONSE: u64 = 64 * 1024;

fn json(response: ureq::Response) -> Result<serde_json::Value, AuthError> {
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_RESPONSE + 1).read_to_end(&mut bytes)
        .map_err(|_| AuthError::new(AuthState::NetworkError))?;
    if bytes.len() as u64 > MAX_RESPONSE { return Err(AuthError::new(AuthState::ProviderError)); }
    serde_json::from_slice(&bytes).map_err(|_| AuthError::new(AuthState::ProviderError))
}

pub(crate) fn post_token(url: &str, form: &[(&str, &str)]) -> Result<Tokens, AuthError> {
    post_token_with_timeout(url, form, TIMEOUT)
}

pub(crate) fn post_token_with_timeout(
    url: &str, form: &[(&str, &str)], timeout: Duration,
) -> Result<Tokens, AuthError> {
    let body = form.iter().filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{}={}", crate::cloud::core_impl::url_enc(key), crate::cloud::core_impl::url_enc(value)))
        .collect::<Vec<_>>().join("&");
    let response = ureq::post(url).timeout(timeout)
        .set("Content-Type", "application/x-www-form-urlencoded").send_string(&body);
    let value = match response {
        Ok(response) => json(response)?,
        Err(ureq::Error::Status(status, response)) => {
            let value = json(response).ok();
            let code = value.as_ref().and_then(|value| value["error"].as_str()).unwrap_or("provider_error");
            return Err(AuthError::provider(status, code));
        }
        Err(_) => return Err(AuthError::new(AuthState::NetworkError)),
    };
    let access_token = value["access_token"].as_str().filter(|token| !token.is_empty())
        .ok_or_else(|| AuthError::new(AuthState::ProviderError))?.to_string();
    let refresh_token = value["refresh_token"].as_str().unwrap_or("").to_string();
    let expires_in = value["expires_in"].as_i64().unwrap_or(3600).max(0);
    Ok(Tokens { access_token, refresh_token,
        expires_at: crate::cloud::core_impl::now_secs().saturating_add(expires_in).saturating_sub(60) })
}

pub(crate) fn refresh(url: &str, cfg: &ClientConfig, token: &str) -> Result<Tokens, AuthError> {
    post_token(url, &[("grant_type", "refresh_token"), ("refresh_token", token),
        ("client_id", &cfg.client_id), ("client_secret", &cfg.client_secret)])
}

pub(crate) fn identify(provider: Provider, token: &str) -> Result<String, AuthError> {
    let url = match provider {
        Provider::GDrive => "https://www.googleapis.com/drive/v3/about?fields=user(permissionId)".to_string(),
    };
    #[cfg(test)]
    let fixture_url = crate::cloud::core_impl::refresh_endpoint(provider)?;
    #[cfg(test)]
    let url = if let Some(authority) = fixture_url.strip_prefix("http://") {
        format!("http://{}/drive/v3/about?fields=user(permissionId)", authority.split('/').next().unwrap_or(""))
    } else { url };
    let response = ureq::get(&url).timeout(TIMEOUT).set("Authorization", &format!("Bearer {token}")).call();
    let value = match response {
        Ok(response) => json(response)?,
        Err(ureq::Error::Status(status, _)) => return Err(AuthError::provider(status, "provider_error")),
        Err(_) => return Err(AuthError::new(AuthState::NetworkError)),
    };
    let id = value["user"]["permissionId"].as_str().filter(|id| !id.is_empty())
        .ok_or_else(|| AuthError::new(AuthState::IdentityUnverified))?;
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(id.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect())
}
