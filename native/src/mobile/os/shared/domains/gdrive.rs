//! `gdrive.*`: the desktop OAuth client config and loopback sign-in. The
//! browser is opened through the host URL opener (`openUrl` event).
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::args::{canceled, invalid, io_error, opt_str, str_arg};
use crate::cloud::{ClientConfig, Provider};
use crate::mobile::{ApiError, Runtime, TaskCtx};

/// How long the sign-in task waits for the browser redirect (api.md §4.6).
const SIGN_IN_WAIT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(250);

pub(super) fn status() -> Result<Value, ApiError> {
    let config = crate::cloud::load_config(Provider::GDrive);
    let client_id = config.client_id.trim().to_string();
    Ok(json!({
        "clientConfigured": !client_id.is_empty(),
        "signedIn": crate::cloud::is_connected(Provider::GDrive),
        "clientId": (!client_id.is_empty()).then_some(client_id),
    }))
}

pub(super) fn configure(args: &Value) -> Result<Value, ApiError> {
    let client_id = str_arg(args, "clientId")?.trim().to_string();
    if client_id.is_empty() {
        return Err(invalid("Client-ID fehlt."));
    }
    let existing = crate::cloud::load_config(Provider::GDrive);
    // A missing secret keeps the stored one; an empty one clears it.
    let config = ClientConfig {
        client_id,
        client_secret: match opt_str(args, "clientSecret") {
            Some(secret) => secret.trim().to_string(),
            None => existing.client_secret,
        },
    };
    if [&config.client_id, &config.client_secret]
        .iter()
        .any(|value| value.chars().any(char::is_control))
    {
        return Err(invalid("Client-ID oder Secret enthält Steuerzeichen."));
    }
    crate::cloud::save_config(Provider::GDrive, &config)
        .map_err(|error| io_error("Google-Drive-Einstellungen speichern", error))?;
    Ok(json!({}))
}

pub(super) fn sign_in(rt: &Runtime) -> Result<Value, ApiError> {
    if crate::cloud::load_config(Provider::GDrive)
        .client_id
        .trim()
        .is_empty()
    {
        return Err(invalid("Bitte zuerst die Client-ID eintragen."));
    }
    let task = rt.spawn_task("oauth", "Google-Drive-Anmeldung".to_string(), sign_in_task);
    Ok(json!({ "taskId": task }))
}

/// The desktop flow blocks until the redirect or its own deadline, so it runs
/// on a helper thread; the task ends on success, cancel or after 180 s.
fn sign_in_task(ctx: &TaskCtx) -> Result<Value, ApiError> {
    ctx.message("Anmeldung im Browser abschließen…");
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("gdrive-sign-in".into())
        .spawn(move || {
            let _ = tx.send(crate::cloud::authorize(Provider::GDrive));
        })
        .map_err(|error| io_error("Anmeldung starten", error))?;
    let deadline = Instant::now() + SIGN_IN_WAIT;
    loop {
        if ctx.cancelled() {
            return Err(canceled("Anmeldung abgebrochen"));
        }
        match rx.recv_timeout(POLL) {
            Ok(Ok(_tokens)) => return Ok(json!({ "signedIn": true })),
            Ok(Err(error)) => return Err(ApiError::new("auth", error)),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return Err(ApiError::new(
                    "internal",
                    "Anmeldung ohne Ergebnis beendet.",
                ))
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= deadline {
            return Err(ApiError::new(
                "network",
                "Zeitüberschreitung: keine Rückmeldung vom Browser.",
            ));
        }
    }
}

pub(super) fn sign_out(rt: &Runtime) -> Result<Value, ApiError> {
    crate::cloud::disconnect(Provider::GDrive).map_err(|error| ApiError::new("internal", error))?;
    // Open Drive sessions still hold the old access token.
    rt.drop_backends(&|key| key.starts_with("gdrive:"));
    Ok(json!({}))
}
