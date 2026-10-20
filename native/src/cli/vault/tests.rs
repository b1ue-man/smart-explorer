//! `se vault` on the command line (stages `vault_cli`, `vault_mcp`; vault
//! plan C7.3, U26-10 to U26-12): the call forms of every command, the exact
//! key sets of the JSON, the small rules of the text (password lines, minutes,
//! groups, the hint behind a question) and the summary `se mcp status` shows.
use clap::Parser;
use serde_json::Value;

use crate::daemon::{CompareHead, CompareItemWire, CompareSideWire, CompareViewFull};
use crate::vault::{RegistryStore, SiteKind, SiteRecord};

use super::{compare, manage, output, sync, VaultArgs, VaultCommand};

/// `se vault` without the root command, so the tests do not depend on the
/// registration in `cli/mod.rs`.
#[derive(Parser)]
struct Probe {
    #[command(flatten)]
    vault: VaultArgs,
}

fn parse(arguments: &[&str]) -> Result<VaultCommand, clap::Error> {
    let mut all = vec!["se"];
    all.extend_from_slice(arguments);
    Probe::try_parse_from(all).map(|probe| probe.vault.command.expect("a subcommand"))
}

const SITE: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn vault_task_cli_parses_every_command() {
    let accepted: [&[&str]; 21] = [
        &["list", "--json"],
        &["show", "Privat", "--json"],
        &["detect", "@nas:/tresore/privat", "--json"],
        &["add", "/tresore/privat", "--label", "Privat", "--json"],
        &[
            "create",
            "/tresore/neu",
            "--shortening-threshold",
            "36",
            "--password-stdin",
        ],
        &["remove", SITE],
        &[
            "unlock",
            "Privat",
            "--password-stdin",
            "--remember",
            "true",
            "--json",
        ],
        &["unlock", "Privat", "--saved-password"],
        &["lock", "Privat", "--keep-unsaved"],
        &["lock", "--all"],
        &["password", "Privat", "--stdin", "--json"],
        &["forget-password", "Privat"],
        &[
            "settings",
            "Privat",
            "--label",
            "Arbeit",
            "--auto-lock-idle-min",
            "off",
            "--lock-on-sleep",
            "false",
            "--lock-on-screen-lock",
            "true",
            "--lock-on-background-min",
            "5",
            "--remember-password",
            "false",
            "--json",
        ],
        &["check", "Privat", "--json"],
        &["sync", "add", "Privat", "--json"],
        &[
            "sync",
            "add",
            "Privat",
            "--with",
            "/mnt/usb/kopie",
            "--name",
            "Kopie",
            "--direction",
            "a2b",
            "--delete-policy",
            "mirror",
            "--compare",
            "checksum",
            "--trigger",
            "interval",
            "--interval-min",
            "15",
            "--enabled",
            "false",
        ],
        &["sync", "add", "Privat", "--with"],
        &["sync", "run", "18a2f0c", "--json"],
        &[
            "compare", "18a2f0c", "--group", "conflict", "--group", "deferred", "--json",
        ],
        &[
            "resolve",
            "18a2f0c",
            "4f1c0a9b2e7d3a55",
            "--action",
            "keep-both",
            "--yes",
        ],
        &[
            "resolve", "18a2f0c", "--all", "--action", "keep-a", "--json",
        ],
    ];
    for arguments in accepted {
        assert!(parse(arguments).is_ok(), "{arguments:?}");
    }
    // `lock` needs a vault or --all; passwords never come as arguments.
    let refused: [&[&str]; 11] = [
        &["lock"],
        &["lock", "Privat", "--all"],
        &["unlock", "Privat", "--password-stdin", "--saved-password"],
        &["unlock", "Privat", "--saved-password", "--remember", "true"],
        &["unlock", "Privat", "--password", "geheim"],
        &["create", "/tresore/neu", "--shortening-threshold", "20"],
        &["create", "/tresore/neu", "--shortening-threshold", "221"],
        &["compare", "18a2f0c", "--group", "nonsense"],
        &["resolve", "18a2f0c", "--action", "keep-a"],
        &[
            "resolve",
            "18a2f0c",
            "4f1c0a9b2e7d3a55",
            "--action",
            "nonsense",
        ],
        &["sync", "add", "Privat", "--direction", "sideways"],
    ];
    for arguments in refused {
        assert!(parse(arguments).is_err(), "{arguments:?} must be refused");
    }
    // Without a subcommand `se vault` lists.
    assert!(Probe::try_parse_from(["se"]).is_ok());
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn wire_side(exists: bool) -> CompareSideWire {
    CompareSideWire {
        exists,
        size: exists.then_some(5),
        mtime_ms: exists.then_some(1_760_000_000_000),
        location: "Tresor A".into(),
    }
}

fn wire_item(path: &str) -> CompareItemWire {
    CompareItemWire {
        item: "4f1c0a9b2e7d3a55".into(),
        group: "conflict".into(),
        kind: "file".into(),
        path: path.into(),
        a: wire_side(true),
        b: wire_side(false),
        actions: vec!["keep-a".into(), "keep-b".into()],
        finding: None,
        note: None,
    }
}

#[test]
fn vault_task_cli_json_keys_exact() {
    // One entry of the vault list.
    let mut record = SiteRecord::new(
        SITE,
        "Privat",
        "/nicht/vorhanden/tresor",
        "vault-id",
        1,
        RegistryStore::open_default().defaults(),
    );
    record.kind = SiteKind::Original;
    let site = output::site_value(&record, &output::context());
    assert_eq!(
        keys(&site),
        [
            "auto_lock_idle_min",
            "cipher",
            "format",
            "kind",
            "label",
            "location",
            "lock_on_background_min",
            "lock_on_screen_lock",
            "lock_on_sleep",
            "mounted",
            "remember_password",
            "site",
            "state",
            "sync_jobs"
        ]
    );
    assert_eq!(site["state"], "locked");
    // Not readable is `null`, never a missing key.
    assert!(site["format"].is_null() && site["cipher"].is_null() && site["mounted"].is_null());
    // An entry of `compare` and `check`: a path that is none is `null`.
    let entry = output::item_value(&wire_item("/Ordner/a.txt"));
    assert_eq!(
        keys(&entry),
        ["a", "actions", "b", "finding", "group", "item", "kind", "note", "path"]
    );
    assert_eq!(
        keys(&entry["a"]),
        ["exists", "location", "mtime_ms", "size"]
    );
    assert!(entry["b"]["size"].is_null() && entry["b"]["mtime_ms"].is_null());
    assert!(output::item_value(&wire_item(""))["path"].is_null());
    // A comparison.
    let mut counts = std::collections::BTreeMap::new();
    counts.insert("conflict".to_string(), 1usize);
    let view = CompareViewFull {
        head: CompareHead {
            job: "18a2f0c".into(),
            site_a: SITE.into(),
            label_a: "Tresor A".into(),
            site_b: "fedcba9876543210fedcba9876543210".into(),
            label_b: "Tresor B".into(),
            revision: 2,
            counts,
            total: 1,
            unavailable: None,
        },
        items: vec![wire_item("/Ordner/a.txt")],
    };
    let json = compare::compare_json(&view);
    assert_eq!(keys(&json), ["counts", "items", "job", "revision", "sides"]);
    assert_eq!(keys(&json["sides"]), ["a", "b"]);
    assert_eq!(keys(&json["sides"]["a"]), ["label", "site"]);
    // The summary of `se mcp status` has numbers only: no path, no name.
    let summary = output::vaults_summary();
    assert_eq!(
        keys(&summary),
        ["entered", "full_list", "jobs", "locked", "unlocked"]
    );
    assert!(summary["full_list"]
        .as_str()
        .is_some_and(|text| text.contains("vault") && text.contains("list")));
    // `key<TAB>value` lines in the order of the keys.
    let lines =
        output::key_lines(&serde_json::json!({ "b": 2, "a": "x", "c": null, "d": ["p", "q"] }));
    assert_eq!(lines, ["a\tx", "b\t2", "c\t-", "d\tp,q"]);
}

#[test]
fn vault_task_cli_small_rules() {
    // The password lines: old first, new second, either line ending.
    assert_eq!(
        manage::two_passwords("alt\nneu").unwrap(),
        ("alt".to_string(), "neu".to_string())
    );
    assert_eq!(
        manage::two_passwords("alt\r\nneu\r\nrest").unwrap(),
        ("alt".to_string(), "neu".to_string())
    );
    assert!(manage::two_passwords("nur eine Zeile").is_err());
    assert!(manage::two_passwords("alt\n").is_err());
    // Minutes and `off`.
    assert_eq!(manage::minutes("off", "x").unwrap(), None);
    assert_eq!(manage::minutes("30", "x").unwrap(), Some(30));
    assert!(manage::minutes("bald", "x").unwrap_err().contains("x"));
    // `all` is every group, none given is left to the worker (no structure).
    let all = compare::groups_of(&["all".to_string()]);
    assert_eq!(all.len(), 9);
    assert!(all.contains(&"structure"));
    assert!(compare::groups_of(&[]).is_empty());
    assert_eq!(
        compare::groups_of(&["conflict".to_string(), "new_a".to_string()]),
        ["conflict", "new_a"]
    );
    // The hint behind a question appears only while it is unanswered.
    let asked = "Die Löschung betrifft den Inhalt eines Ordners. Trotzdem übernehmen?".to_string();
    assert!(compare::with_hint(asked.clone(), false).ends_with("Mit --yes bestätigen."));
    assert_eq!(compare::with_hint(asked.clone(), true), asked);
    assert_eq!(
        compare::with_hint("anderer Fehler".into(), false),
        "anderer Fehler"
    );
    // Weekdays and the places `sync add --with` takes.
    assert_eq!(sync::weekday_mask("mon,wed").unwrap(), 0b101);
    assert_eq!(sync::weekday_mask("daily").unwrap(), 0);
    assert!(sync::weekday_mask("someday").is_err());
    for place in [
        "sftp://host/pfad",
        "@nas:/tresor",
        "/mnt/usb",
        "./kopie",
        "C:/Tresor",
    ] {
        assert!(sync::looks_like_location(place), "{place}");
    }
    assert!(!sync::looks_like_location("Privat"));
    // The words of a refused request come from the table of codes.
    let error = crate::daemon::VaultIpcError {
        message: "Tresor x ist gesperrt".into(),
        code: Some("vault_locked".into()),
    };
    let text = output::vault_error(&error, "Privat");
    assert!(text.contains("se vault unlock Privat"), "{text}");
    let plain = crate::daemon::VaultIpcError {
        message: "Speicherort nicht erreichbar".into(),
        code: None,
    };
    assert_eq!(
        output::vault_error(&plain, "Privat"),
        "Speicherort nicht erreichbar"
    );
    assert_eq!(output::clean("a\tb\nc\r"), "a b c ");
    assert_eq!(output::side_text(&wire_side(true)), "+:5");
    assert_eq!(output::side_text(&wire_side(false)), "-:?");
}

#[test]
fn vault_task_cli_completion_offers_sites_and_jobs() {
    // A site of the vault list is offered with its label as help.
    let store = RegistryStore::open_default();
    let id = crate::vault::new_site_id().expect("site ID");
    let label = format!("Vervollstaendigung-{id}");
    let record = SiteRecord::new(
        &id,
        &label,
        &format!("/nicht/vorhanden/{id}"),
        "",
        1,
        store.defaults(),
    );
    store.add(&record).expect("site entered");
    let sites = super::complete::site_candidates();
    store.remove(&id).expect("site removed");
    let site = sites
        .iter()
        .find(|candidate| candidate.get_value() == std::ffi::OsStr::new(&id))
        .expect("the site is offered");
    assert!(site
        .get_help()
        .is_some_and(|help| help.to_string().contains(&label)));
    // A vault job is offered with its name; a general job is not.
    let mut job = crate::syncjobs::SyncJob::new_vault(
        "Vervollstaendigung".into(),
        SITE,
        "fedcba9876543210fedcba9876543210",
    );
    job.id = format!(
        "vaulttask{}c{}",
        std::process::id(),
        id.get(..8).unwrap_or("x")
    );
    crate::syncjobs::upsert(&job).expect("vault job saved");
    let jobs = super::complete::job_candidates();
    crate::syncjobs::remove(&job.id).expect("vault job removed");
    assert!(jobs
        .iter()
        .any(|candidate| candidate.get_value() == std::ffi::OsStr::new(&job.id)));
}

#[test]
fn vault_task_mcp_locked_vault_names_the_unlock_command() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let store = RegistryStore::at(
        dir.path().join("vaults"),
        crate::vault::LockSettings::desktop(),
    );
    let id = crate::vault::new_site_id().expect("site ID");
    let record = SiteRecord::new(&id, "Privat", "/x", "", 1, store.defaults());
    store.add(&record).expect("site entered");
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    // `se cat vault://…` of a vault that is not unlocked.
    let error = crate::vault::open_location_in(&store, &id, "/Akten/a.txt", &resolve, None)
        .err()
        .expect("locked");
    assert_eq!(error.code(), Some("vault_locked"));
    let unlock = format!("se vault unlock {id}");
    assert!(error.to_string().contains(&unlock), "{error}");
    // The table of codes says the same, with the vault filled in.
    let refused = crate::daemon::VaultIpcError {
        message: error.to_string(),
        code: Some("vault_locked".into()),
    };
    assert!(output::vault_error(&refused, &id).contains(&unlock));
}
