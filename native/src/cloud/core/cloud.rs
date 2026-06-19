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
use std::io::{Read, Write};
use std::time::Duration;

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
    pub fn label(self) -> &'static str {
        match self {
            Provider::GDrive => "Google Drive",
        }
    }
    fn auth_url(self) -> &'static str {
        match self {
            Provider::GDrive => "https://accounts.google.com/o/oauth2/v2/auth",
        }
    }
    fn token_url(self) -> &'static str {
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
    disconnect, is_configured, is_connected, load_config, refresh_token, save_config,
    store_refresh_token,
};

// ── PKCE ─────────────────────────────────────────────────────────────────────

fn random_b64(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    // OS CSPRNG; fall back to a time/pid mix only if it somehow fails (never on
    // supported platforms) so we degrade rather than panic.
    if getrandom::getrandom(&mut buf).is_err() {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ (std::process::id() as u128);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((seed >> (i % 16 * 8)) as u8) ^ (i as u8).wrapping_mul(31);
        }
    }
    URL_SAFE_NO_PAD.encode(&buf)
}

fn sha256_b64url(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    URL_SAFE_NO_PAD.encode(h.finalize())
}

/// (verifier, challenge) for PKCE S256.
fn pkce_pair() -> (String, String) {
    let verifier = random_b64(48); // 64 url-safe chars, within RFC 7636 limits
    let challenge = sha256_b64url(&verifier);
    (verifier, challenge)
}

fn build_auth_url(
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
fn url_enc(s: &str) -> String {
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
fn parse_redirect(request_line: &str) -> Option<(String, String)> {
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
                let h = u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'%');
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

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn post_token(url: &str, form: &[(&str, &str)]) -> Result<Tokens, String> {
    // Skip empty fields: a public PKCE client has no secret, and sending
    // `client_secret=` makes Google answer 400 invalid_client.
    let body: String = form
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", url_enc(k), url_enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    let text = match ureq::post(url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
    {
        Ok(r) => r.into_string().map_err(|e| e.to_string())?,
        // ureq surfaces 4xx/5xx as Err(Status) — read Google's JSON error so the
        // real cause (redirect_uri_mismatch, invalid_grant, invalid_client, …)
        // is shown instead of a bare "status code 400".
        Err(ureq::Error::Status(code, r)) => {
            let raw = r.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| {
                    let e = v.get("error").and_then(|x| x.as_str()).unwrap_or("");
                    let d = v
                        .get("error_description")
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    let combined = format!("{} {}", e, d);
                    let combined = combined.trim().to_string();
                    if combined.is_empty() {
                        None
                    } else {
                        Some(combined)
                    }
                })
                .unwrap_or(raw);
            return Err(format!("HTTP {}: {}", code, detail));
        }
        Err(e) => return Err(e.to_string()),
    };
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let access = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            v.get("error_description")
                .or_else(|| v.get("error"))
                .and_then(|x| x.as_str())
                .unwrap_or("kein access_token in der Antwort")
                .to_string()
        })?
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let expires_in = v.get("expires_in").and_then(|x| x.as_i64()).unwrap_or(3600);
    Ok(Tokens {
        access_token: access,
        refresh_token: refresh,
        expires_at: now_secs() + expires_in - 60,
    })
}

/// Run the full interactive authorize flow (opens the browser, catches the
/// loopback redirect, exchanges the code). Blocking — call off the UI thread.
/// On success the refresh token is stored in the keyring.
pub fn authorize(p: Provider) -> Result<Tokens, String> {
    let cfg = load_config(p);
    if cfg.client_id.trim().is_empty() {
        return Err("Kein OAuth Client-ID konfiguriert".into());
    }
    // Loopback listener on an ephemeral port.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{}", port);
    let (verifier, challenge) = pkce_pair();
    let state = random_b64(16);
    let url = build_auth_url(p, &cfg.client_id, &redirect, &challenge, &state);

    super::os::open_url(&url);

    // Wait (with timeout) for the single redirect request.
    listener.set_nonblocking(false).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    listener.set_ttl(64).ok();
    let (mut stream, _) = accept_with_deadline(&listener, deadline)?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first = req.lines().next().unwrap_or("");
    let (code, got_state) =
        parse_redirect(first).ok_or_else(|| "Keine Autorisierung erhalten".to_string())?;
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
          <html><body style='font-family:sans-serif'><h3>Smart Explorer</h3>\
          <p>Anmeldung abgeschlossen. Sie koennen dieses Fenster schliessen.</p>\
          </body></html>",
    );
    if got_state != state {
        return Err("Sicherheitsfehler (state stimmt nicht)".into());
    }

    // Exchange the code for tokens.
    let tokens = post_token(
        p.token_url(),
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", &cfg.client_id),
            ("client_secret", &cfg.client_secret),
            ("code_verifier", &verifier),
        ],
    )?;
    if !tokens.refresh_token.is_empty() {
        store_refresh_token(p, &tokens.refresh_token);
    }
    Ok(tokens)
}

/// Exchange the stored refresh token for a fresh access token. Blocking.
pub fn refresh_access(p: Provider) -> Result<Tokens, String> {
    let cfg = load_config(p);
    let refresh = refresh_token(p).ok_or_else(|| "Nicht verbunden".to_string())?;
    let mut t = post_token(
        p.token_url(),
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &cfg.client_id),
            ("client_secret", &cfg.client_secret),
        ],
    )?;
    // Google omits refresh_token on refresh — keep the stored one.
    if t.refresh_token.is_empty() {
        t.refresh_token = refresh;
    } else {
        store_refresh_token(p, &t.refresh_token);
    }
    Ok(t)
}

fn accept_with_deadline(
    listener: &std::net::TcpListener,
    deadline: std::time::Instant,
) -> Result<(std::net::TcpStream, std::net::SocketAddr), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    loop {
        match listener.accept() {
            Ok(pair) => {
                let _ = listener.set_nonblocking(false);
                return Ok(pair);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err("Zeitüberschreitung bei der Anmeldung".into());
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (v, c) = pkce_pair();
        assert!(!v.is_empty() && !c.is_empty());
        assert!(!v.contains('=') && !c.contains('='));
        assert!(!v.contains('+') && !c.contains('/'));
        let (v2, _) = pkce_pair();
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
