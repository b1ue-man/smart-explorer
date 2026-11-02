//! K20 of the vault plan: the transfer log keeps no vault cleartext. Fields
//! of a vault side are sealed with the vault's key, withheld when that is
//! not possible; transfers without a vault keep their lines unchanged.
use super::*;
use crate::bisync::test_remote::FakeRemote;
use crate::vfs::Scheme;

const SITE: &str = "0123456789abcdef0123456789abcdef";
const MARKER: &str = "Geheimakte4d1";

/// A sealer of the test: the result never contains its input.
fn fake_sealer() -> FieldSealer {
    Arc::new(|site: &str, items: &[&str]| {
        Ok(items
            .iter()
            .map(|item| format!("versiegelt:{site}:{}", item.len()))
            .collect())
    })
}

fn failing_sealer() -> FieldSealer {
    Arc::new(|_: &str, _: &[&str]| Err("Tresor gesperrt".to_string()))
}

fn lines(log: &IssueLog) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(log.log_path().expect("log file")).expect("log text");
    assert!(!text.contains(MARKER), "cleartext in the log: {text}");
    text.lines()
        .map(|line| serde_json::from_str(line).expect("JSON line"))
        .collect()
}

#[test]
fn vault_task_transfer_issue_path_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let path = format!("/{MARKER}/Plan.txt");
    let message = format!("{MARKER}/Plan.txt: Zugriff verweigert");

    // A vault side whose site is known: path and message sealed.
    let storage = tempfile::tempdir().unwrap();
    let vault = FakeRemote::new(storage.path(), "tresor-k20").with_scheme(Scheme::Vault);
    crate::vault::note_vault_backend(&vault, SITE);
    let sealed = IssueLog::for_sides_with(
        1,
        dir.path().join("a"),
        Side::Remote(&vault),
        Side::Local,
        fake_sealer(),
    );
    sealed.push(&path, &message);
    let written = lines(&sealed);
    assert_eq!(written[0]["vault"], SITE);
    assert!(written[0]["path_sealed"]
        .as_str()
        .is_some_and(|text| text.starts_with("versiegelt:")));
    assert!(written[0].get("path").is_none());
    // The transfer list in memory keeps the entry as it is.
    assert_eq!(sealed.shown().0[0].path, path);

    // Sealing not possible (vault locked, no worker): withheld.
    let locked = IssueLog::for_sides_with(
        2,
        dir.path().join("b"),
        Side::Local,
        Side::Remote(&vault),
        failing_sealer(),
    );
    locked.push(&path, &message);
    let written = lines(&locked);
    assert_eq!(written[0]["path"], "");
    assert_eq!(written[0]["message"], crate::vault::FIELDS_WITHHELD);

    // A vault backend of unknown site: withheld, never cleartext.
    let other = FakeRemote::new(storage.path(), "fremd-k20").with_scheme(Scheme::Vault);
    let unknown = IssueLog::for_sides_with(
        3,
        dir.path().join("c"),
        Side::Remote(&other),
        Side::Local,
        fake_sealer(),
    );
    unknown.push(&path, &message);
    assert_eq!(lines(&unknown)[0]["vault"], "");

    // A local path in a vault drive is sealed even without a vault side.
    let drive = dir.path().join("laufwerk");
    crate::vault::publish_vault_mount(SITE, &drive);
    let local = IssueLog::for_sides_with(
        4,
        dir.path().join("d"),
        Side::Local,
        Side::Local,
        fake_sealer(),
    );
    let inside = drive.join(MARKER).to_string_lossy().into_owned();
    local.push(&inside, "kaputt");
    local.push("/home/anna/frei.txt", "kaputt");
    let written = lines(&local);
    crate::vault::withdraw_vault_mount(SITE, &drive);
    assert_eq!(written[0]["vault"], SITE);
    // Transfers without a vault keep their lines exactly as before.
    assert_eq!(
        written[1],
        serde_json::json!({ "path": "/home/anna/frei.txt", "message": "kaputt" })
    );
    let plain = IssueLog::new(5, dir.path().join("e"));
    plain.push("/src/f0", "kaputt");
    let text = std::fs::read_to_string(plain.log_path().unwrap()).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(text.trim_end()).unwrap(),
        serde_json::json!({ "path": "/src/f0", "message": "kaputt" })
    );
}
