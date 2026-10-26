//! Token-free public diagnostics and the private, atomically replaced session.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    #[default]
    Disconnected,
    LegacyUnverified,
    Connected,
    ReauthorizeRequired,
    ClientChanged,
    IdentityConflict,
    IdentityUnverified,
    NetworkError,
    StorageError,
    ProviderError,
    OperationSuperseded,
}

impl AuthState {
    pub fn message(self) -> &'static str {
        match self {
            Self::Disconnected => "Nicht angemeldet",
            Self::LegacyUnverified => "Gespeicherte Anmeldung: Kontobindung noch nicht bestätigt",
            Self::Connected => "Angemeldet",
            Self::ReauthorizeRequired => "Anmeldung erneuern (invalid_grant). Projekt möglicherweise im Testmodus; Veröffentlichungsstatus ist nicht bestätigt.",
            Self::ClientChanged => "OAuth-Client-ID fehlt oder wurde geändert; mit demselben Google-Konto erneut anmelden",
            Self::IdentityConflict => "Drive-Konto wurde gewechselt; gespeicherte Ordnerbindung gehört zu einem anderen Konto",
            Self::IdentityUnverified => "Google-Drive-Konto wurde nicht eindeutig bestätigt; vorhandene Anmeldung und Ordnerbindungen bleiben erhalten",
            Self::NetworkError => "Vorhandene Anmeldung bleibt gespeichert; Netzwerk oder Google-Dienst ist vorübergehend nicht erreichbar",
            Self::StorageError => "OAuth-Anmeldespeicher oder Cloud-Konfiguration konnte nicht gelesen oder geschrieben werden",
            Self::OperationSuperseded => "Anmeldung wurde durch eine neuere Anmeldung, Abmeldung oder Clientänderung überholt; erneut anmelden",
            Self::ProviderError => "Google hat die OAuth-Anfrage abgelehnt; Clientkonfiguration prüfen",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderFailure {
    pub http_status: Option<u16>,
    pub code: String,
}

#[derive(Clone, Debug)]
pub struct AuthError {
    pub state: AuthState,
    pub provider: Option<ProviderFailure>,
}

impl AuthError {
    pub(crate) fn new(state: AuthState) -> Self {
        Self {
            state,
            provider: None,
        }
    }

    pub(crate) fn provider(status: u16, code: &str) -> Self {
        // Never persist descriptions, response bodies or an untrusted error value.
        let code = match code {
            "invalid_grant"
            | "invalid_client"
            | "unauthorized_client"
            | "access_denied"
            | "invalid_request"
            | "invalid_scope"
            | "temporarily_unavailable"
            | "server_error"
            | "admin_policy_enforced"
            | "rateLimitExceeded"
            | "userRateLimitExceeded"
            | "sharingRateLimitExceeded"
            | "quotaExceeded" => code,
            _ => "provider_error",
        };
        let state = if code == "invalid_grant" {
            AuthState::ReauthorizeRequired
        } else if status >= 500
            || status == 429
            || matches!(code, "temporarily_unavailable" | "server_error")
            || (status == 403
                && matches!(
                    code,
                    "rateLimitExceeded"
                        | "userRateLimitExceeded"
                        | "sharingRateLimitExceeded"
                        | "quotaExceeded"
                ))
        {
            AuthState::NetworkError
        } else {
            AuthState::ProviderError
        };
        Self {
            state,
            provider: Some(ProviderFailure {
                http_status: Some(status),
                code: code.into(),
            }),
        }
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.state.message())?;
        if let Some(error) = &self.provider {
            write!(f, " [OAuth {}", error.code)?;
            if let Some(status) = error.http_status {
                write!(f, "; HTTP {status}")?;
            }
            f.write_str("]")?;
        }
        Ok(())
    }
}
impl std::error::Error for AuthError {}

#[derive(Clone, Debug, Serialize)]
pub struct AuthStatus {
    pub state: AuthState,
    pub message: String,
    pub client_configured: bool,
    pub client_id: Option<String>,
    pub signed_in: bool,
    pub account_key: Option<String>,
    pub generation: u64,
    pub login_at: Option<i64>,
    pub last_refresh_at: Option<i64>,
    pub client_changed_at: Option<i64>,
    pub provider_error: Option<ProviderFailure>,
    /// No API in the current flow confirms the user's Cloud project status.
    pub project_status: &'static str,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    pub version: u8,
    pub provider: String,
    pub client_id: String,
    pub account_key: Option<String>,
    pub generation: u64,
    pub refresh_token: String,
    pub state: AuthState,
    pub login_at: Option<i64>,
    pub last_refresh_at: Option<i64>,
    pub client_changed_at: Option<i64>,
    pub provider_error: Option<ProviderFailure>,
}

impl Session {
    pub(crate) fn advance(&mut self) -> Result<(), AuthError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| AuthError::new(AuthState::StorageError))?;
        Ok(())
    }

    pub(crate) fn check_account(
        &self,
        account: &str,
        expected: Option<&str>,
    ) -> Result<(), AuthError> {
        if account.is_empty() {
            return Err(AuthError::new(AuthState::IdentityUnverified));
        }
        if expected.is_some_and(|key| key != account)
            || self
                .account_key
                .as_deref()
                .is_some_and(|key| key != account)
        {
            return Err(AuthError::new(AuthState::IdentityConflict));
        }
        Ok(())
    }
}
