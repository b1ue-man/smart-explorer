//! K08 of the vault plan: the spool of a copy with a vault on either side
//! holds only ciphertext under a key used once; plain copies keep their
//! plain spool.
use std::io::Read;

use super::*;
use crate::bisync::test_remote::FakeRemote;
use crate::vfs::{LocalBackend, Scheme};

const MARKER: &[u8] = b"Klartextmarke-K08-41b";

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn sealed_spools() -> usize {
    SEALED_SPOOLS.with(|count| count.get())
}

#[test]
fn vault_task_copy_spool_sealed_when_vault_involved() {
    // The spool itself: ciphertext on disk, the cleartext back on reading.
    let mut content = MARKER.repeat(5000);
    content.extend_from_slice(b"Ende");
    let mut spool = EphemeralSpool::new().unwrap();
    assert_eq!(
        spool.fill(&mut content.as_slice()).unwrap(),
        content.len() as u64
    );
    let stored = spool.stored().unwrap();
    assert!(!contains(&stored, MARKER), "no cleartext in the spool file");
    let mut back = Vec::new();
    spool.reader().unwrap().read_to_end(&mut back).unwrap();
    assert_eq!(back, content);

    // Copies between backends: a vault side seals the spool, others do not.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_string_lossy().replace('\\', "/");
    // `/data` of a fake remote is the folder it is made on.
    std::fs::write(dir.path().join("quelle.txt"), &content).unwrap();
    let vault_view = FakeRemote::new(dir.path(), "tresor").with_scheme(Scheme::Vault);
    let local = LocalBackend::new("/");
    let before = sealed_spools();
    let copied = crate::vfs::copy_between(
        &vault_view,
        "/data/quelle.txt",
        &local,
        &format!("{root}/ziel.txt"),
    )
    .unwrap();
    assert_eq!(copied, content.len() as u64);
    assert_eq!(std::fs::read(dir.path().join("ziel.txt")).unwrap(), content);
    assert_eq!(sealed_spools(), before + 1, "vault source: sealed spool");
    let plain = FakeRemote::new(dir.path(), "eben");
    crate::vfs::copy_between(
        &plain,
        "/data/quelle.txt",
        &local,
        &format!("{root}/ziel2.txt"),
    )
    .unwrap();
    assert_eq!(
        sealed_spools(),
        before + 1,
        "no vault: plain spool as before"
    );
    assert_eq!(
        std::fs::read(dir.path().join("ziel2.txt")).unwrap(),
        content
    );
}
