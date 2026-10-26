//! Serialized OAuth transactions, shared by GUI, daemon, CLI and mobile.
use super::{oauth_http, session_store as store, shared};
use crate::cloud::auth_state::Session;
use crate::cloud::{AuthError, AuthState, AuthStatus, ClientConfig, Provider, Tokens};
use std::sync::Mutex;

static RUNTIME_FAILURE: Mutex<Option<AuthError>> = Mutex::new(None);

fn now() -> i64 { crate::cloud::core_impl::now_secs() }
fn runtime_error(error: &AuthError) {
    if let Ok(mut value) = RUNTIME_FAILURE.lock() { *value = Some(error.clone()); }
}
fn commit(provider: Provider, session: &mut Session) -> Result<(), AuthError> {
    persist(provider, session, true)
}
fn persist(provider: Provider, session: &mut Session, tracked: bool) -> Result<(), AuthError> {
    // Pure diagnostics do not supersede an in-flight explicit browser login.
    if tracked || session.generation == 0 { session.advance()?; }
    if let Err(error) = store::write(provider, session, tracked) {
        runtime_error(&error);
        return Err(error);
    }
    if let Ok(mut value) = RUNTIME_FAILURE.lock() { *value = None; }
    Ok(())
}

fn config(provider: Provider) -> Result<ClientConfig, AuthError> {
    shared::load_config_checked(provider).map_err(|_| store::storage_error())
}
fn session(provider: Provider, client: &str) -> Result<Session, AuthError> {
    Ok(store::read(provider)?.unwrap_or_else(|| store::empty(provider, client)))
}

pub(super) fn begin_authorize(provider: Provider) -> Result<(ClientConfig, u64), AuthError> {
    let needs_binding = {
        let _guard = store::lock(provider)?;
        let cfg = config(provider)?;
        let record = session(provider, &cfg.client_id)?;
        let has_token = if record.generation == 0 {
            store::read_legacy(provider)?.is_some_and(|token| !token.is_empty())
        } else { !record.refresh_token.is_empty() };
        record.account_key.is_none() && has_token && !cfg.client_id.is_empty()
            && record.client_id == cfg.client_id
            && !matches!(record.state, AuthState::ReauthorizeRequired | AuthState::ProviderError | AuthState::IdentityConflict)
    };
    if needs_binding {
        // First confirm a still usable legacy grant before a browser can select
        // a different account. Never try its token against multiple client IDs.
        if let Err(error) = refresh(provider, &crate::cloud::core_impl::refresh_endpoint(provider)?, None, |token| oauth_http::identify(provider, token)) {
            if !matches!(error.state, AuthState::ReauthorizeRequired | AuthState::ProviderError) {
                return Err(error);
            }
            // An unusable legacy grant has no provable principal: explicitly
            // renew through PKCE, preserving every existing Drive root ID.
        }
    }
    let _guard = store::lock(provider)?;
    let cfg = config(provider)?;
    let record = session(provider, &cfg.client_id)?;
    Ok((cfg, record.generation))
}

pub(super) fn finish_authorize(
    provider: Provider, cfg: &ClientConfig, generation: u64, tokens: Tokens,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Tokens, AuthError> {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) { return Err(AuthError::new(AuthState::OperationSuperseded)); }
    if tokens.refresh_token.trim().is_empty() { return Err(AuthError::new(AuthState::ProviderError)); }
    let _guard = store::lock(provider)?;
    let current = config(provider)?;
    let mut record = session(provider, &current.client_id)?;
    if current.client_id != cfg.client_id || current.client_secret != cfg.client_secret
        || generation != record.generation {
        return Err(AuthError::new(AuthState::OperationSuperseded));
    }
    let account = oauth_http::identify(provider, &tokens.access_token)?;
    if let Err(error) = record.check_account(&account, None) {
        // Preserve the existing successful grant and its tokens on a rejected login.
        runtime_error(&error);
        return Err(error);
    }
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err(AuthError::new(AuthState::OperationSuperseded));
    }
    record.client_id = cfg.client_id.clone();
    record.account_key = Some(account);
    record.refresh_token = tokens.refresh_token.clone();
    record.login_at = Some(now());
    record.last_refresh_at = None;
    record.state = AuthState::Connected;
    record.provider_error = None;
    commit(provider, &mut record)?;
    // The committed record is authoritative; a leftover legacy token is never read again.
    let _ = crate::creds::delete_secret_checked(&store::legacy_key(provider));
    Ok(tokens)
}

pub(crate) fn refresh(
    provider: Provider,
    token_url: &str,
    expected_account: Option<&str>,
    identify: impl FnOnce(&str) -> Result<String, AuthError>,
) -> Result<Tokens, AuthError> {
    let _guard = store::lock(provider)?;
    let cfg = config(provider)?;
    let mut record = session(provider, &cfg.client_id)?;
    if cfg.client_id.trim().is_empty() || record.client_id != cfg.client_id {
        return Err(AuthError::new(AuthState::ClientChanged));
    }
    if expected_account.zip(record.account_key.as_deref()).is_some_and(|(a, b)| a != b) {
        return Err(AuthError::new(AuthState::IdentityConflict));
    }
    if matches!(record.state, AuthState::ReauthorizeRequired | AuthState::ProviderError | AuthState::IdentityConflict) {
        return Err(AuthError { state: record.state, provider: record.provider_error });
    }
    let previous = if record.generation == 0 {
        // A legacy grant is adopted once, only after this exact client accepts it
        // and the provider confirms its one principal under the same lock.
        store::read_legacy(provider)?.unwrap_or_default()
    } else { record.refresh_token.clone() };
    if previous.is_empty() { return Err(AuthError::new(AuthState::Disconnected)); }
    record.refresh_token = previous.clone();
    let mut tokens = match oauth_http::refresh(token_url, &cfg, &previous) {
        Ok(tokens) => tokens,
        Err(error) => {
            record_failure(provider, &mut record, &error, false)?;
            return Err(error);
        }
    };
    if tokens.refresh_token.is_empty() { tokens.refresh_token = previous; }
    let account = match identify(&tokens.access_token) {
        Ok(account) => account,
        Err(error) => {
            // The provider may already have invalidated the previous token.
            // Preserve rotation even when the independent identity read failed;
            // no access token or new principal becomes usable on this path.
            let rotated = record.refresh_token != tokens.refresh_token;
            record.refresh_token = tokens.refresh_token;
            record_failure(provider, &mut record, &error, rotated)?;
            return Err(error);
        }
    };
    if let Err(error) = record.check_account(&account, expected_account) {
        record_failure(provider, &mut record, &error, false)?;
        return Err(error);
    }
    record.account_key = Some(account);
    record.refresh_token = tokens.refresh_token.clone();
    record.last_refresh_at = Some(now());
    record.state = AuthState::Connected;
    record.provider_error = None;
    commit(provider, &mut record)?;
    let _ = crate::creds::delete_secret_checked(&store::legacy_key(provider));
    Ok(tokens)
}

fn record_failure(provider: Provider, record: &mut Session, error: &AuthError, rotated: bool) -> Result<(), AuthError> {
    record.state = error.state;
    record.provider_error = error.provider.clone();
    // A pure network diagnostic keeps the grant usable and must not wake jobs.
    // Rotation or a terminal authentication transition still invalidates users.
    persist(provider, record, rotated || !matches!(error.state, AuthState::NetworkError | AuthState::IdentityUnverified))
}

pub(crate) fn status(provider: Provider) -> AuthStatus {
    let mut value = AuthStatus {
        state: AuthState::Disconnected, message: String::new(), client_configured: false,
        client_id: None, signed_in: false, account_key: None, generation: 0,
        login_at: None, last_refresh_at: None, client_changed_at: None,
        provider_error: None, project_status: "unconfirmed",
    };
    let result = (|| {
        // A complete secure-store record is atomic; status must remain responsive
        // while another process holds the refresh lock across network requests.
        let cfg = config(provider)?;
        value.client_configured = !cfg.client_id.is_empty();
        value.client_id = value.client_configured.then_some(cfg.client_id.clone());
        if let Some(record) = store::read(provider)? {
            value.state = if record.client_id != cfg.client_id { AuthState::ClientChanged } else { record.state };
            value.signed_in = !record.refresh_token.is_empty() && !matches!(value.state,
                AuthState::ReauthorizeRequired | AuthState::ClientChanged | AuthState::IdentityConflict | AuthState::ProviderError);
            value.account_key = record.account_key;
            value.generation = record.generation;
            value.login_at = record.login_at;
            value.last_refresh_at = record.last_refresh_at;
            value.client_changed_at = record.client_changed_at;
            value.provider_error = record.provider_error;
        } else if store::read_legacy(provider)?.is_some_and(|token| !token.is_empty()) {
            value.state = AuthState::LegacyUnverified;
            value.signed_in = true;
        }
        Ok::<_, AuthError>(())
    })();
    if let Err(error) = result { value.state = error.state; value.signed_in = false; }
    if let Ok(failure) = RUNTIME_FAILURE.lock() {
        if let Some(error) = failure.as_ref() {
            value.state = error.state;
            value.provider_error = error.provider.clone();
            if error.state == AuthState::StorageError { value.signed_in = false; }
        }
    }
    value.message = value.state.message().into();
    value
}

/// A cached bearer never bypasses a later disconnect, client switch or invalid grant.
pub(crate) fn validate_account(provider: Provider, account: &str) -> Result<(), AuthError> {
    let cfg = config(provider)?;
    let record = store::read(provider)?.ok_or_else(|| AuthError::new(AuthState::Disconnected))?;
    if record.account_key.as_deref() != Some(account) { return Err(AuthError::new(AuthState::IdentityConflict)); }
    if record.client_id != cfg.client_id { return Err(AuthError::new(AuthState::ClientChanged)); }
    if record.refresh_token.is_empty() { return Err(AuthError::new(AuthState::Disconnected)); }
    if matches!(record.state, AuthState::ReauthorizeRequired | AuthState::IdentityConflict | AuthState::ProviderError) {
        return Err(AuthError { state: record.state, provider: record.provider_error });
    }
    Ok(())
}

pub(crate) fn success_revision(provider: Provider) -> Option<i64> {
    let record = store::read(provider).ok()??;
    if record.state != AuthState::Connected { return None; }
    record.login_at.into_iter().chain(record.last_refresh_at).max()
}

pub(super) fn disconnect(provider: Provider) -> Result<(), AuthError> {
    let _guard = store::lock(provider)?;
    let cfg = config(provider)?;
    let mut record = session(provider, &cfg.client_id)?;
    // Erase the legacy grant before creating the authoritative disconnected tombstone.
    crate::creds::delete_secret_checked(&store::legacy_key(provider)).map_err(|_| store::storage_error())?;
    record.refresh_token.clear();
    record.state = AuthState::Disconnected;
    record.provider_error = None;
    // Keep the confirmed principal: provider-wide saved roots still belong to it.
    commit(provider, &mut record)
}

pub(super) fn store_import(provider: Provider, token: &str) -> Result<(), AuthError> {
    if token.trim().is_empty() { return Err(AuthError::new(AuthState::ProviderError)); }
    let _guard = store::lock(provider)?;
    let cfg = config(provider)?;
    let mut record = session(provider, &cfg.client_id)?;
    // Imported tokens cannot change an already confirmed principal. The
    // verifier checks the retained account before any access token escapes.
    if record.account_key.is_none() && record.refresh_token.is_empty() { record.client_id = cfg.client_id.clone(); }
    if record.client_id != cfg.client_id { return Err(AuthError::new(AuthState::ClientChanged)); }
    record.refresh_token = token.into();
    record.state = AuthState::LegacyUnverified;
    record.provider_error = None;
    commit(provider, &mut record)
}

pub(super) fn refresh_token(provider: Provider) -> Result<Option<String>, AuthError> {
    let _guard = store::lock(provider)?;
    match store::read(provider)? {
        Some(record) => Ok((!record.refresh_token.is_empty()).then_some(record.refresh_token)),
        None => store::read_legacy(provider),
    }
}

pub(super) fn config_changed(provider: Provider, cfg: &ClientConfig) -> std::io::Result<()> {
    let _guard = store::lock(provider).map_err(std::io::Error::other)?;
    let previous = shared::load_config_checked(provider)?;
    let mut record = store::read(provider).map_err(std::io::Error::other)?
        .unwrap_or_else(|| store::empty(provider, &previous.client_id));
    if record.generation == 0 {
        record.refresh_token = store::read_legacy(provider).map_err(std::io::Error::other)?.unwrap_or_default();
        if record.refresh_token.is_empty() { record.client_id = cfg.client_id.clone(); }
        else { record.state = AuthState::LegacyUnverified; }
    }
    // Advance before changing the config, so even a failed config write or a
    // client A -> B -> A transition invalidates pending browser callbacks.
    if previous.client_id != cfg.client_id || previous.client_secret != cfg.client_secret {
        record.client_changed_at = Some(now());
        if record.client_id == cfg.client_id && record.state == AuthState::ProviderError {
            record.state = AuthState::LegacyUnverified;
            record.provider_error = None;
        }
        commit(provider, &mut record).map_err(std::io::Error::other)?;
    }
    shared::save_config_unlocked(provider, cfg)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_clear_runtime_failure() {
    if let Ok(mut value) = RUNTIME_FAILURE.lock() { *value = None; }
}

#[cfg(test)]
#[path = "batches_auth_task_tests.rs"]
mod batches_auth_task_tests;
