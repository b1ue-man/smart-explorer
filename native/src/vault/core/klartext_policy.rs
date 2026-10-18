//! The plaintext boundary of vaults (vault plan C4, VLT-M6): for every place
//! where se keeps data (K01–K48) the rule for the contents of an unlocked
//! vault, and the pure decisions the places share. No I/O and no platform
//! code; the hooks live at the places themselves and in
//! `vault/os/shared/local_store.rs`, `ephemeral_spool.rs` and
//! `app/os/shared/vault_open.rs`.
use std::path::Path;

use super::locator::{is_site_id, vault_location, VaultLocator};

/// What a place may hold of a vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// Only encrypted (vault content format, or a key used once).
    Encrypted,
    /// Encrypted, and removed when the vault locks.
    EncryptedDroppedOnLock,
    /// Path fields sealed with the purpose key `log-fields`.
    PathFieldsSealed,
    /// Never: vault places are left out or refused.
    Excluded,
    /// Only the vault root `vault://<site-id>/`; places inside sealed in
    /// `<AD>/vaults/<site-id>.local`.
    RootOnly,
    /// A cleartext copy only after the user agreed (fallback copy).
    ConsentOnly,
    /// Only the encrypted storage (cipher paths and bytes).
    CiphertextOnly,
    /// Only in memory, dropped when the vault locks.
    MemoryOnly,
    /// Only when the user asks for it (remembered password).
    OnRequest,
    /// Holds no vault data; unchanged.
    NoVaultData,
    /// Outside se (system caches): counter-measures and notes.
    OutsideSe,
}

/// One place of C4 with its rule and where it is enforced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SinkRule {
    pub(crate) id: &'static str,
    pub(crate) place: &'static str,
    pub(crate) rule: Rule,
    pub(crate) enforced: &'static str,
}

const fn sink(
    id: &'static str,
    place: &'static str,
    rule: Rule,
    enforced: &'static str,
) -> SinkRule {
    SinkRule {
        id,
        place,
        rule,
        enforced,
    }
}

/// Every place K01–K48 of C4, one entry each.
pub(crate) const SINKS: [SinkRule; 48] = [
    sink(
        "K01",
        "Laufwerks-Spool",
        Rule::Encrypted,
        "vault/os/shared/sealed_spool.rs (S2-V2)",
    ),
    sink("K02", "Clean-Cache", Rule::EncryptedDroppedOnLock, "S2-V10"),
    sink(
        "K03",
        "journal.jsonl, stages.json",
        Rule::PathFieldsSealed,
        "vault/os/shared/journal_sealer.rs (S2-V3)",
    ),
    sink(
        "K04",
        "private-runtime/",
        Rule::NoVaultData,
        "nur Dokany-DLL",
    ),
    sink(
        "K05",
        "open-temp",
        Rule::ConsentOnly,
        "app/os/shared/vault_open.rs, clipboard_provide.rs, clipboard_materialize.rs",
    ),
    sink("K06", "session.pid", Rule::NoVaultData, "–"),
    sink(
        "K07",
        "Wiederherstellungsmanifest",
        Rule::ConsentOnly,
        "nur Rückfallkopien (app/os/shared/vault_open.rs)",
    ),
    sink(
        "K08",
        "anonyme Kopierdatei copy_transfer",
        Rule::Encrypted,
        "vfs/os/shared/copy_transfer.rs, vault/os/shared/ephemeral_spool.rs",
    ),
    sink(
        "K09",
        "analysis-cache/",
        Rule::Excluded,
        "share/os/shared/storage_roots.rs excluded()",
    ),
    sink(
        "K10",
        "host-trash",
        Rule::Excluded,
        "folgt aus K09 und delete_disposition Permanent",
    ),
    sink(
        "K11",
        "Ordnerindex Desktop",
        Rule::Excluded,
        "folder_index/os/persistence.rs",
    ),
    sink(
        "K12",
        "Ordnerindex Android",
        Rule::Excluded,
        "folder_index/os/persistence.rs, mobile/os/shared/index.rs",
    ),
    sink(
        "K13",
        "favorites.txt, dir_sort.tsv",
        Rule::RootOnly,
        "connect/os/shared/location_prefs.rs",
    ),
    sink(
        "K14",
        "recent.txt, mobile/recent.json",
        Rule::RootOnly,
        "app/core/prefs_tabs.rs, mobile/os/shared/places.rs",
    ),
    sink("K15", "connections.txt", Rule::NoVaultData, "–"),
    sink(
        "K16",
        "secrets-v1",
        Rule::OnRequest,
        "vault/os/shared/remember.rs",
    ),
    sink(
        "K17",
        "Windows-Anmeldeinformationen",
        Rule::OnRequest,
        "vault/os/shared/remember.rs",
    ),
    sink("K18", "shell-sessions/", Rule::NoVaultData, "–"),
    sink("K19", "cloud/<provider>.cfg", Rule::NoVaultData, "–"),
    sink(
        "K20",
        "transfer-logs/",
        Rule::PathFieldsSealed,
        "transfer/os/shared/engine/issues.rs",
    ),
    sink(
        "K21",
        "reclaim/actions.jsonl",
        Rule::PathFieldsSealed,
        "app/core/reclaim_core.rs",
    ),
    sink(
        "K22",
        "share_server.txt",
        Rule::NoVaultData,
        "Signalserver-Adresse",
    ),
    sink("K23", "lan_uplink/", Rule::NoVaultData, "Uplink-Anfrage"),
    sink(
        "K24",
        "crash.log",
        Rule::CiphertextOnly,
        "Result statt Panik; Site-ID statt Pfad",
    ),
    sink(
        "K25",
        "update_*.txt, staged_update.json",
        Rule::NoVaultData,
        "–",
    ),
    sink(
        "K26",
        "<AD>/mobile/",
        Rule::RootOnly,
        "edits.json (K44), recent.json (K14)",
    ),
    sink(
        "K27",
        "Mount-Registry registry-2.json",
        Rule::CiphertextOnly,
        "MountSource::Vault { site_id } (S2-V1)",
    ),
    sink(
        "K28",
        "Jobdateien",
        Rule::CiphertextOnly,
        "syncjobs/os/shared/vault_jobs_dir.rs (vault-store://)",
    ),
    sink("K29", "jobs.tsv", Rule::NoVaultData, "Altformat"),
    sink("K30", "results.tsv", Rule::NoVaultData, "Altformat"),
    sink(
        "K31",
        "sync_state.sqlite",
        Rule::CiphertextOnly,
        "Tresor-Sync auf Chiffrat (C5.1)",
    ),
    sink(
        "K32",
        "Baseline",
        Rule::CiphertextOnly,
        "Tresor-Sync auf Chiffrat (C5.1)",
    ),
    sink(
        "K33",
        "Ersetzungsjournal",
        Rule::CiphertextOnly,
        "Tresor-Sync auf Chiffrat (C5.1)",
    ),
    sink(
        "K34",
        "Checkpoint-Journal",
        Rule::CiphertextOnly,
        "Tresor-Sync auf Chiffrat (C5.1)",
    ),
    sink(
        "K35",
        "job-logs/",
        Rule::CiphertextOnly,
        "Tresor-Sync auf Chiffrat (C5.1)",
    ),
    sink(
        "K36",
        "versions_<pair>/",
        Rule::CiphertextOnly,
        "versions_location = appdata (C5.6)",
    ),
    sink(
        "K37",
        ".se-versions/",
        Rule::CiphertextOnly,
        "Tresor-Jobs nie Auto (C5.6)",
    ),
    sink(
        "K38",
        "Merge-Eingaben",
        Rule::CiphertextOnly,
        "Vergleichsmodus übergibt Chiffrat (C6.4)",
    ),
    sink(
        "K39",
        "Konfliktkopien",
        Rule::CiphertextOnly,
        "keine automatischen; „Beide behalten“ mit Chiffratnamen (C6.4)",
    ),
    sink(
        "K40",
        "Kopierstufen",
        Rule::CiphertextOnly,
        "Chiffratstufen (C2.6)",
    ),
    sink("K41", "nested-mounts.json", Rule::CiphertextOnly, "–"),
    sink(
        "K42",
        "catchup.last.json",
        Rule::CiphertextOnly,
        "Annahme A9",
    ),
    sink("K43", "legacy-baseline-jobs/", Rule::CiphertextOnly, "–"),
    sink(
        "K44",
        "Android <cache>/open/, edits.json",
        Rule::Excluded,
        "mobile/os/shared/edits.rs (vault_use_provider)",
    ),
    sink(
        "K45",
        "Zwischenspeicher fremder Programme",
        Rule::OutsideSe,
        "vault/os/linux_thumbnails.rs, Hinweise S2",
    ),
    sink(
        "K46",
        "Speicher von se",
        Rule::MemoryOnly,
        "LockListener, Sperren",
    ),
    sink(
        "K47",
        "Auslagerungs-/Ruhezustandsdatei",
        Rule::OutsideSe,
        "Sperren beim Ruhezustand",
    ),
    sink("K48", "IPC-Loopback", Rule::MemoryOnly, "Token"),
];

/// The rule of place `id` (`"K01"` … `"K48"`).
pub(crate) fn rule_for(id: &str) -> Option<&'static SinkRule> {
    SINKS.iter().find(|sink| sink.id == id)
}

/// K05 (2), (3): why „Für andere Programme bereitstellen“ and the clipboard
/// with long relative paths refuse vault sources.
pub(crate) const PROVIDE_REFUSED: &str = "Tresorinhalte werden nicht als Klartextkopie \
bereitgestellt: als Laufwerk einhängen oder in einen gewählten Ordner kopieren (Export)";

/// K44: the Bridge code of `fs.fetch`, `fs.open` and `fs.materialize` for
/// vault locations, and its text.
pub(crate) const USE_PROVIDER_CODE: &str = "vault_use_provider";
pub(crate) const USE_PROVIDER_TEXT: &str = "Tresordateien öffnen andere Apps über den \
Dokumentanbieter von se, ohne Klartextkopie im Zwischenspeicher";

/// K20: what an issue line of a vault says when its fields cannot be sealed.
pub(crate) const FIELDS_WITHHELD: &str =
    "Eintrag eines Tresors – Pfad und Meldung nur bei entsperrtem Tresor in der Übertragungsliste";

/// A place inside a vault: the site as written and the cleartext path
/// (`/` for the vault root). `None` for every other location.
pub(crate) fn split_place(location: &str) -> Option<(String, String)> {
    let locator = VaultLocator::parse(location.trim_start()).ok()?;
    let path = if locator.path.is_empty() {
        "/".to_string()
    } else {
        locator.path
    };
    Some((locator.site, path))
}

/// The only global form of places inside a vault (K13, K14).
pub(crate) fn root_place(site: &str) -> String {
    vault_location(site, "/")
}

/// Whether `place` is the root of a vault with a site ID.
pub(crate) fn root_site(place: &str) -> Option<String> {
    let (site, path) = split_place(place)?;
    (path == "/" && is_site_id(&site)).then_some(site)
}

/// Whether `path` lies at or below `root` (whole components).
pub(crate) fn within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_task_policy_every_sink_has_rule() {
        // Every place K01–K48 of C4 has exactly one rule and a named place
        // that enforces it.
        for number in 1..=48 {
            let id = format!("K{number:02}");
            let matches = SINKS.iter().filter(|sink| sink.id == id).count();
            assert_eq!(matches, 1, "{id}");
            let sink = rule_for(&id).expect("rule");
            assert!(!sink.place.is_empty() && !sink.enforced.is_empty(), "{id}");
        }
        assert!(rule_for("K49").is_none());
        // The places this package enforces carry the rules of C4.
        for (id, rule) in [
            ("K05", Rule::ConsentOnly),
            ("K08", Rule::Encrypted),
            ("K09", Rule::Excluded),
            ("K11", Rule::Excluded),
            ("K12", Rule::Excluded),
            ("K13", Rule::RootOnly),
            ("K14", Rule::RootOnly),
            ("K20", Rule::PathFieldsSealed),
            ("K21", Rule::PathFieldsSealed),
            ("K28", Rule::CiphertextOnly),
            ("K44", Rule::Excluded),
        ] {
            assert_eq!(rule_for(id).map(|sink| sink.rule), Some(rule), "{id}");
        }
        // Places inside a vault keep only their root globally.
        let site = "0123456789abcdef0123456789abcdef";
        let place = format!("vault://{site}/Geheim/Plan.txt");
        assert_eq!(
            split_place(&place),
            Some((site.to_string(), "/Geheim/Plan.txt".to_string()))
        );
        assert_eq!(root_place(site), format!("vault://{site}/"));
        assert_eq!(root_site(&root_place(site)).as_deref(), Some(site));
        assert!(root_site(&place).is_none());
        assert!(split_place("/home/anna/Geheim").is_none());
        assert!(split_place(&format!("vault-store://{site}")).is_none());
        assert!(within(
            Path::new("/run/user/1000/smart-explorer/Tresore/A/x"),
            Path::new("/run/user/1000/smart-explorer/Tresore")
        ));
        assert!(!within(
            Path::new("/run/user/1000/smart-explorer/Tresorex"),
            Path::new("/run/user/1000/smart-explorer/Tresore")
        ));
    }
}
