//! Android's switch of a plaintext Share server address to the encrypted
//! address of the same server (TODO `DIREKT-INTERNET`), the rule of the
//! desktop worker (`crate::share::encrypted_upgrade`): `tcp://host:port`
//! gives an `http://` relay without QUIC address discovery, so this device
//! never learns its public address and every connection across the internet
//! goes over the relay.
//!
//! The desktop worker tries it while it reads the server file. The embedded
//! worker reads that file under its state lock, and the Share page reads it
//! under `share_state`'s lock, so the network attempt (TLS and WebSocket
//! handshake) runs here on its own thread at bridge start. Only the swap
//! takes a lock: the file is replaced when it still holds the address that
//! was tested, the cached entry when it is unchanged too, and the worker
//! reloads afterwards.
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::share_state::{reconfigure, server_path, with_state};
use crate::mobile::Runtime;
use crate::share::server_address::SignalServerConfig;

static STARTED: AtomicBool = AtomicBool::new(false);
/// Serializes every write of the server file in this process (`share.setServer`
/// and this switch), so the switch never overwrites a newer user entry.
static SERVER_FILE: Mutex<()> = Mutex::new(());

/// The lock every writer of the server file holds while it writes.
pub(super) fn server_file_lock() -> MutexGuard<'static, ()> {
    SERVER_FILE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the one attempt of this process in the background.
pub(super) fn start(rt: &'static Runtime) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("share-server-tls".into())
        .spawn(move || run(rt));
    if let Err(error) = spawned {
        rt.record_error(
            "share",
            &format!("Prüfung der verschlüsselten Server-Adresse nicht gestartet: {error}"),
        );
    }
}

fn run(rt: &'static Runtime) {
    let path = server_path();
    let Ok(tested) = std::fs::read_to_string(&path) else {
        return;
    };
    // Network, outside every lock.
    let Some(encrypted) = crate::share::encrypted_upgrade(&tested) else {
        return;
    };
    let swapped = {
        let _file = server_file_lock();
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if !same_server(&current, &tested) {
            false
        } else if let Err(error) = write_atomic(&path, &encrypted) {
            rt.record_error(
                "share",
                &format!("Verschlüsselte Server-Adresse speichern: {error}"),
            );
            false
        } else {
            with_state(|state| {
                let unchanged = match state.server.as_deref() {
                    None => true,
                    Some(cached) => same_server(cached, &tested),
                };
                if unchanged {
                    state.server = Some(encrypted.clone());
                }
                state.notice(format!(
                    "Share-Server-Adresse auf {encrypted} umgestellt (verschlüsselt, \
                     Adresserkennung für Direktwege)"
                ));
            });
            true
        }
    };
    if swapped {
        reconfigure(rt);
    }
}

/// Whether two stored server entries name the same server address: equal
/// canonical forms (a legacy value without scheme equals its `tcp://` form,
/// which the worker may have written meanwhile, B21).
pub(super) fn same_server(left: &str, right: &str) -> bool {
    match (
        SignalServerConfig::parse_stored(left),
        SignalServerConfig::parse_stored(right),
    ) {
        (Ok(left), Ok(right)) => left.canonical() == right.canonical(),
        _ => left.trim() == right.trim(),
    }
}

fn write_atomic(path: &std::path::Path, value: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(value.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::same_server;

    #[test]
    fn direkt_internet_task_server_swap_only_for_the_tested_address() {
        // The worker's canonical rewrite of a legacy value is the same server.
        assert!(same_server(
            "server.example:51820",
            "tcp://server.example:51820"
        ));
        assert!(same_server(
            "tcp://server.example:51820\n",
            "tcp://server.example:51820"
        ));
        // A user entry written meanwhile is never replaced.
        assert!(!same_server(
            "tcp://other.example:51820",
            "tcp://server.example:51820"
        ));
        assert!(!same_server(
            "wss://server.example:51820",
            "tcp://server.example:51820"
        ));
        assert!(!same_server("", "tcp://server.example:51820"));
    }
}
