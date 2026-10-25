//! Cloud / OAuth foundation (#19), provider-agnostic with Google Drive wired
//! first. This slice owns the *auth*: a PKCE loopback OAuth2 flow, the client-ID
//! config, and token storage (refresh token in the OS keyring). The Drive
//! `Backend` itself lands in a follow-up slice (`gdrive.rs`) and consumes
//! `access_token()` from here.
//!
//! Why PKCE + loopback: a desktop app can't keep a real client secret, so we use
//! the Authorization-Code-with-PKCE flow and catch the redirect on an ephemeral
//! `127.0.0.1` port — no inbound firewall holes, no embedded secret relied upon.
//! See docs/CLOUD_OAUTH_PLAN.md.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    GDrive,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::GDrive => "gdrive",
        }
    }
    fn auth_url(self) -> &'static str {
        match self {
            Provider::GDrive => "https://accounts.google.com/o/oauth2/v2/auth",
        }
    }
    pub(crate) fn token_url(self) -> &'static str {
        match self {
            Provider::GDrive => "https://oauth2.googleapis.com/token",
        }
    }
    fn scope(self) -> &'static str {
        match self {
            // Full Drive access so two-way sync can read AND write.
            Provider::GDrive => "https://www.googleapis.com/auth/drive",
        }
    }
}

// ── config (client id/secret) + token storage ───────────────────────────────

/// The OAuth client the user registered in their own cloud project.
#[derive(Clone, Default)]
pub struct ClientConfig {
    pub client_id: String,
    /// Google issues a "secret" even for desktop clients; it isn't truly secret
    /// but the token endpoint still wants it. Empty is allowed (pure PKCE).
    pub client_secret: String,
}

pub use super::os::shared::{
    disconnect, is_connected, load_config, load_config_checked, refresh_token,
    refresh_token_checked, save_config, store_refresh_token,
};

// ── PKCE ─────────────────────────────────────────────────────────────────────

pub(crate) fn random_b64(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).map_err(|_| "Sichere OAuth-Zufallswerte konnten nicht erzeugt werden".to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(&buf))
}

fn sha256_b64url(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    URL_SAFE_NO_PAD.encode(h.finalize())
}

/// (verifier, challenge) for PKCE S256.
pub(crate) fn pkce_pair() -> Result<(String, String), String> {
    let verifier = random_b64(48)?; // 64 url-safe chars, within RFC 7636 limits
    let challenge = sha256_b64url(&verifier);
    Ok((verifier, challenge))
}

pub(crate) fn build_auth_url(
    p: Provider,
    client_id: &str,
    redirect: &str,
    challenge: &str,
    state: &str,
) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &code_challenge={}&code_challenge_method=S256&state={}\
         &access_type=offline&prompt=consent",
        p.auth_url(),
        url_enc(client_id),
        url_enc(redirect),
        url_enc(p.scope()),
        url_enc(challenge),
        url_enc(state),
    )
}

/// Minimal application/x-www-form-urlencoded component encoder.
pub(crate) fn url_enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Pull `code` and `state` out of an HTTP request line ("GET /?code=…&state=… HTTP/1.1").
pub(crate) fn parse_redirect(request_line: &str) -> Option<(String, String)> {
    let path = request_line.split_whitespace().nth(1)?; // "/?code=..&state=.."
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for kv in query.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            match k {
                "code" => code = Some(url_dec(v)),
                "state" => state = Some(url_dec(v)),
                _ => {}
            }
        }
    }
    Some((code?, state.unwrap_or_default()))
}

fn url_dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3]).ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok()).unwrap_or(b'%');
                out.push(h);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

// ── token exchange / refresh ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64, // unix seconds
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Run the full interactive authorize flow (opens the browser, catches the
/// loopback redirect, exchanges the code). Blocking — call off the UI thread.
/// On success the refresh token is stored in the keyring.
pub fn authorize(p: Provider) -> Result<Tokens, String> {
    authorize_cancellable(p, &std::sync::atomic::AtomicBool::new(false))
}

/// A canceled mobile task must not persist a late browser result.
pub fn authorize_cancellable(
    p: Provider, cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Tokens, String> {
    super::os::oauth_flow::authorize(p, cancelled)
}

/// Exchange the stored refresh token for a fresh access token. Blocking.
pub fn refresh_access(p: Provider) -> Result<Tokens, String> {
    refresh_access_with_identity(p, None, |token| super::os::oauth_http::identify(p, token))
        .map_err(|error| error.to_string())
}

/// Every caller supplies its existing principal check to the same transaction.
/// The process/file lock stays held from credential read through verification
/// and the single atomic record replacement.
pub(crate) fn refresh_access_with_identity(
    p: Provider,
    expected_account: Option<&str>,
    identify: impl FnOnce(&str) -> Result<String, super::AuthError>,
) -> Result<Tokens, super::AuthError> {
    super::os::coordinator::refresh(p, &refresh_endpoint(p)?, expected_account, identify)
}

pub(crate) fn refresh_endpoint(p: Provider) -> Result<String, super::AuthError> {
    #[cfg(test)]
    if let Some(url) = sync_reliability_task_refresh_hook::endpoint()
        .map_err(|_| super::AuthError::new(super::AuthState::StorageError))? {
        return Ok(url);
    }
    Ok(p.token_url().into())
}

pub(crate) fn validate_auth_account(p: Provider, account: &str) -> Result<(), super::AuthError> {
    super::os::coordinator::validate_account(p, account)
}

pub fn auth_status(p: Provider) -> super::AuthStatus { super::os::coordinator::status(p) }

pub(crate) fn auth_success_revision(p: Provider) -> Option<i64> {
    super::os::coordinator::success_revision(p)
}

#[cfg(test)]
#[path = "sync_reliability_task_refresh_hook.rs"]
mod sync_reliability_task_refresh_hook;
#[cfg(test)]
pub use sync_reliability_task_refresh_hook::{test_refresh_endpoint, TestRefreshEndpoint};

#[cfg(test)]
#[path = "startup_regression_task_tests.rs"]
mod startup_regression_task_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            sha256_b64url(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn pkce_pair_is_unpadded_and_distinct() {
        let (v, c) = pkce_pair().unwrap();
        assert!(!v.is_empty() && !c.is_empty());
        assert!(!v.contains('=') && !c.contains('='));
        assert!(!v.contains('+') && !c.contains('/'));
        let (v2, _) = pkce_pair().unwrap();
        assert_ne!(v, v2, "verifier must be random per call");
    }

    #[test]
    fn auth_url_has_required_params() {
        let u = build_auth_url(
            Provider::GDrive,
            "cid.apps.googleusercontent.com",
            "http://127.0.0.1:1234",
            "CHAL",
            "STATE",
        );
        assert!(u.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(u.contains("response_type=code"));
        assert!(u.contains("code_challenge=CHAL"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(u.contains("state=STATE"));
        assert!(u.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1234"));
        assert!(u.contains("access_type=offline"));
    }

    #[test]
    fn token_exchange_has_a_bounded_response_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(150));
            drop(stream);
        });

        let started = std::time::Instant::now();
        let error = super::super::os::oauth_http::post_token_with_timeout(
            &url,
            &[("grant_type", "refresh_token")],
            Duration::from_millis(30),
        )
        .err()
        .expect("blackholed token exchange must time out");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!error.to_string().is_empty());
        server.join().unwrap();
    }

    #[test]
    fn parse_redirect_extracts_code_and_state() {
        let line = "GET /?code=4%2F0Ab-xyz&state=abc123 HTTP/1.1";
        let (code, state) = parse_redirect(line).unwrap();
        assert_eq!(code, "4/0Ab-xyz");
        assert_eq!(state, "abc123");
    }

    #[test]
    fn url_enc_dec_roundtrip() {
        let s = "a b/c?d=e&f";
        assert_eq!(url_dec(&url_enc(s)), s);
    }
}
