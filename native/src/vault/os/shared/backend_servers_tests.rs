//! Vaults on real servers (VLT-M14, stage `vault-backends`, job linux-core).
//! The stage starts the owned servers of
//! `native/sync-reliability-providers.py` (SFTP, SFTP with the agent, FTP,
//! FTPS, SMB, WebDAV and a Direct Share pair through the candidate Share
//! server) and hands their connections over in the manifest
//! `SE_SYNC_PROVIDER_MANIFEST`, with the isolated profile the host keys were
//! confirmed in. Both tests run the scenario of
//! `backend_servers_support::exercise` and need that manifest, so they are
//! `#[ignore]`d outside the stage.
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::connect::sync_reliability_task_provider_fixture as fixture;

use super::backend_servers_support::{
    exercise, new_marker, panic_text, random_hex, require_servers,
};

/// The backends of the matrix by the provider names of the manifest: SFTP,
/// SFTP with the agent, FTP, FTPS, SMB and WebDAV.
const MATRIX: [&str; 6] = ["sftp", "agent", "ftp", "ftps", "smb", "webdav"];
/// The Direct Share pair through the candidate Share server.
const PAIR: &str = "direct";

/// The scenario in a fresh folder of the provider.
fn run_on(provider: &fixture::Provider, kind: &str, marker: &str) {
    let (backend, base) = provider.open(&format!("{kind}-{}", random_hex(4)));
    exercise(&backend, &base, marker);
}

#[test]
#[ignore = "braucht die Testserver der Stufe vault-backends (Manifest SE_SYNC_PROVIDER_MANIFEST aus native/sync-reliability-providers.py)"]
fn vault_task_backends_provider_matrix() {
    require_servers();
    let providers = fixture::providers();
    let marker = new_marker();
    let (mut passed, mut missing, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for name in MATRIX {
        let Some(provider) = providers.iter().find(|provider| provider.name == name) else {
            missing.push(name);
            continue;
        };
        match catch_unwind(AssertUnwindSafe(|| {
            run_on(provider, "vault-matrix", &marker)
        })) {
            Ok(()) => passed.push(name),
            Err(payload) => failed.push(format!("{name}: {}", panic_text(&*payload))),
        }
    }
    println!("Tresor auf {} geprüft", passed.join(", "));
    assert!(
        failed.is_empty(),
        "Tresor-Matrix: {} von {} Backends gescheitert:\n{}",
        failed.len(),
        MATRIX.len(),
        failed.join("\n")
    );
    // A backend the manifest lacks was not reached: the others passed, this
    // one is "not run" and never counts as passed.
    if !missing.is_empty() {
        panic!(
            "nicht ausgeführt: im Manifest der Testserver fehlen {} (geprüft: {})",
            missing.join(", "),
            passed.join(", ")
        );
    }
}

#[test]
#[ignore = "braucht das Direct-Share-Paar der Stufe vault-backends (Manifest SE_SYNC_PROVIDER_MANIFEST, Kandidaten-Share-Server)"]
fn vault_task_backends_share_pair() {
    require_servers();
    let providers = fixture::providers();
    let Some(provider) = providers.iter().find(|provider| provider.name == PAIR) else {
        panic!(
            "nicht ausgeführt: das Direct-Share-Paar \"{PAIR}\" fehlt im Manifest der Testserver"
        );
    };
    run_on(provider, "vault-share-pair", &new_marker());
}
