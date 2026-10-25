//! Host browser and loopback callback; the coordinator alone commits credentials.
use crate::cloud::core_impl::{build_auth_url, parse_redirect, pkce_pair, random_b64};
use crate::cloud::{Provider, Tokens};
use std::io::{Read, Write};
use std::time::Duration;

pub(crate) fn authorize(p: Provider, cancelled: &std::sync::atomic::AtomicBool) -> Result<Tokens, String> {
    let (cfg, generation) = super::coordinator::begin_authorize(p).map_err(|error| error.to_string())?;
    if cfg.client_id.trim().is_empty() {
        return Err("Kein OAuth Client-ID konfiguriert".into());
    }
    // Loopback listener on an ephemeral port.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{}", port);
    let (verifier, challenge) = pkce_pair()?;
    let state = random_b64(16)?;
    let url = build_auth_url(p, &cfg.client_id, &redirect, &challenge, &state);

    super::open_url(&url)?;

    // Wait (with timeout) for the single redirect request.
    listener.set_nonblocking(false).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    listener.set_ttl(64).ok();
    let (mut stream, _) = accept_with_deadline(&listener, deadline, cancelled)?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first = req.lines().next().unwrap_or("");
    let (code, got_state) =
        parse_redirect(first).ok_or_else(|| "Keine Autorisierung erhalten".to_string())?;
    if got_state != state {
        return Err("Sicherheitsfehler (state stimmt nicht)".into());
    }

    // Exchange the code for tokens.
    let tokens = super::oauth_http::post_token(
        &crate::cloud::core_impl::refresh_endpoint(p).map_err(|error| error.to_string())?,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", &cfg.client_id),
            ("client_secret", &cfg.client_secret),
            ("code_verifier", &verifier),
        ],
    ).map_err(|error| error.to_string())?;
    let tokens = super::coordinator::finish_authorize(p, &cfg, generation, tokens, cancelled)
        .map_err(|error| error.to_string())?;
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
          <html><body style='font-family:sans-serif'><h3>Smart Explorer</h3>\
          <p>Anmeldung abgeschlossen. Sie koennen dieses Fenster schliessen.</p>\
          </body></html>",
    );
    Ok(tokens)
}

fn accept_with_deadline(
    listener: &std::net::TcpListener,
    deadline: std::time::Instant,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<(std::net::TcpStream, std::net::SocketAddr), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    loop {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) { return Err("Anmeldung abgebrochen".into()); }
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
