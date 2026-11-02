//! Stage `vault_cli` (vault plan VLT-M9, C7.3, U26-10 to U26-12): every `se
//! vault` command that prints JSON, run as the built `se` in an isolated
//! environment, prints exactly the keys of the contract, and the exit codes
//! the contract names. One vault goes through the whole life: create,
//! detect, add, list, show, settings, password, unlock, check (healthy and
//! with a finding), a vault job with its copy, comparison and resolution.
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::task_env::{assert_keys, copy_dir, field, text, TaskEnv, PASSWORD};
use super::task_flow::{item_at, settle, Pair};

/// An entry of `list`.
const SITE_KEYS: [&str; 14] = [
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
    "sync_jobs",
];
/// `show` and `settings`: the entry of `list` and three more.
const SHOW_KEYS: [&str; 17] = [
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
    "recovery_key_note",
    "recovery_key_supported",
    "remember_password",
    "shortening_threshold",
    "site",
    "state",
    "sync_jobs",
];
/// An entry of `compare` and `check`.
const ITEM_KEYS: [&str; 9] = [
    "a", "actions", "b", "finding", "group", "item", "kind", "note", "path",
];
/// One side of an entry.
const SIDE_KEYS: [&str; 4] = ["exists", "location", "mtime_ms", "size"];
/// The `counts` of `compare`: every group, also with zero.
const GROUP_KEYS: [&str; 9] = [
    "changed_a",
    "changed_b",
    "conflict",
    "deferred",
    "deleted_a",
    "deleted_b",
    "new_a",
    "new_b",
    "structure",
];
/// `sync run`.
const RUN_KEYS: [&str; 6] = [
    "conflicts",
    "copied_a_to_b",
    "copied_b_to_a",
    "deferred",
    "deleted",
    "errors",
];

/// The only content folder (`d/XX/YYYY…`) of a vault that holds files in its
/// root and no folder: the root folder.
fn root_content_dir(vault: &Path) -> PathBuf {
    let mut found = Vec::new();
    for outer in std::fs::read_dir(vault.join("d")).expect("d/ des Tresors") {
        let outer = outer.expect("Eintrag").path();
        if !outer.is_dir() {
            continue;
        }
        for inner in std::fs::read_dir(outer).expect("d/XX") {
            let inner = inner.expect("Eintrag").path();
            if inner.is_dir() {
                found.push(inner);
            }
        }
    }
    assert_eq!(found.len(), 1, "ein Inhaltsordner (die Wurzel): {found:?}");
    found.remove(0)
}

/// Every entry of a `compare` has the keys of the contract.
fn assert_items(compared: &Value, what: &str) {
    let items = compared["items"].as_array().expect("items ist eine Liste");
    for item in items {
        assert_keys(item, &ITEM_KEYS, what);
        assert_keys(&item["a"], &SIDE_KEYS, what);
        assert_keys(&item["b"], &SIDE_KEYS, what);
        assert!(item["actions"].is_array(), "{what}: {item}");
    }
}

#[test]
#[ignore = "braucht die gebaute se in SMART_EXPLORER_VAULT_SE_BIN (Stufe vault-cli, env=SMART_EXPLORER_VAULT_SE_BIN={se})"]
fn vault_task_cli_every_command_json_keys() {
    let env = TaskEnv::isolated();
    let local = |name: &str, content: &str| -> PathBuf {
        let path = env.path(&format!("input/{name}"));
        std::fs::write(&path, content).expect("lokale Datei");
        path
    };

    // create
    let created = env.create_json("orte/arbeit", "Arbeit");
    assert_keys(&created, &["site", "vault_id"], "vault create");
    let site = field(&created, "site");
    assert_eq!(site.len(), 32, "{created}");
    let vault_id = field(&created, "vault_id");
    let vault_dir = env.path("orte/arbeit");

    // detect: a vault, and a folder that holds none
    let detect_keys = ["is_vault", "format", "key_source", "vault_id"];
    let detected = env
        .se(&["vault", "detect", text(&vault_dir), "--json"])
        .ok()
        .json();
    assert_keys(&detected, &detect_keys, "vault detect");
    assert_eq!(detected["is_vault"], true);
    assert_eq!(detected["format"], 8);
    assert_eq!(detected["key_source"], "masterkeyfile");
    assert_eq!(detected["vault_id"], vault_id.as_str());
    let empty = env.path("orte/leer");
    std::fs::create_dir_all(&empty).expect("leerer Ordner");
    let none = env
        .se(&["vault", "detect", text(&empty), "--json"])
        .ok()
        .json();
    assert_keys(&none, &detect_keys, "vault detect (kein Tresor)");
    assert_eq!(none["is_vault"], false);
    assert!(
        none["format"].is_null() && none["key_source"].is_null() && none["vault_id"].is_null(),
        "{none}"
    );

    // add: the same vault in a second folder
    let copy_path = env.path("orte/kopie");
    copy_dir(&vault_dir, &copy_path);
    let added = env
        .se(&[
            "vault",
            "add",
            text(&copy_path),
            "--label",
            "Kopie",
            "--json",
        ])
        .ok()
        .json();
    assert_keys(&added, &["site", "label", "vault_id"], "vault add");
    assert_eq!(added["label"], "Kopie");
    assert_eq!(added["vault_id"], vault_id.as_str());
    let copy_site = field(&added, "site");

    // list: the two sites are one vault
    let listed = env.se(&["vault", "list", "--json"]).ok().json();
    assert_keys(&listed, &["vaults"], "vault list");
    let groups = listed["vaults"].as_array().expect("vaults ist eine Liste");
    assert_eq!(groups.len(), 1, "ein Tresor, eine Gruppe: {listed}");
    assert_keys(&groups[0], &["vault_id", "sites"], "vault list: Gruppe");
    assert_eq!(groups[0]["vault_id"], vault_id.as_str());
    let sites = groups[0]["sites"].as_array().expect("sites ist eine Liste");
    assert_eq!(sites.len(), 2, "{listed}");
    for entry in sites {
        assert_keys(entry, &SITE_KEYS, "vault list: Ort");
        assert_eq!(entry["state"], "locked");
        assert_eq!(entry["format"], 8);
        assert!(
            matches!(entry["cipher"].as_str(), Some("SIV_GCM" | "SIV_CTRMAC")),
            "{entry}"
        );
        assert_eq!(entry["kind"], "original");
        assert!(entry["mounted"].is_null(), "{entry}");
        assert_eq!(entry["sync_jobs"], json!([]));
    }

    // show and settings: the entry of the list and three more
    let shown = env
        .se(&["vault", "show", site.as_str(), "--json"])
        .ok()
        .json();
    assert_keys(&shown, &SHOW_KEYS, "vault show");
    assert_eq!(shown["site"], site.as_str());
    assert_eq!(shown["label"], "Arbeit");
    assert_eq!(shown["location"], text(&vault_dir));
    assert_eq!(shown["recovery_key_supported"], false);
    assert!(
        shown["recovery_key_note"]
            .as_str()
            .is_some_and(|note| !note.is_empty()),
        "{shown}"
    );
    assert!(
        shown["shortening_threshold"].is_null(),
        "ohne Sitzung kein Wert: {shown}"
    );
    let changed = env
        .se(&[
            "vault",
            "settings",
            site.as_str(),
            "--label",
            "Arbeit 2",
            "--auto-lock-idle-min",
            "20",
            "--lock-on-sleep",
            "false",
            "--lock-on-screen-lock",
            "true",
            "--json",
        ])
        .ok()
        .json();
    assert_keys(&changed, &SHOW_KEYS, "vault settings");
    assert_eq!(changed["label"], "Arbeit 2");
    assert_eq!(changed["auto_lock_idle_min"], 20);
    assert_eq!(changed["lock_on_sleep"], false);
    assert_eq!(changed["lock_on_screen_lock"], true);

    // password: old password in line 1, new one in line 2 (on the copy, so
    // the vault of the following steps keeps its password)
    let both = format!("{PASSWORD}\nNeues-Passwort-2026\n");
    let password = env
        .se_in(
            &["vault", "password", copy_site.as_str(), "--stdin", "--json"],
            &both,
        )
        .ok()
        .json();
    assert_keys(&password, &["site", "backup"], "vault password");
    assert_eq!(password["site"], copy_site.as_str());
    assert!(password["backup"].is_boolean(), "{password}");

    // unlock
    let unlocked = env
        .se_in(
            &[
                "vault",
                "unlock",
                site.as_str(),
                "--password-stdin",
                "--json",
            ],
            PASSWORD,
        )
        .ok()
        .json();
    assert_keys(&unlocked, &["site", "state"], "vault unlock");
    assert_eq!(unlocked["site"], site.as_str());
    assert_eq!(unlocked["state"], "unlocked");
    let shown = env
        .se(&["vault", "show", site.as_str(), "--json"])
        .ok()
        .json();
    assert_keys(&shown, &SHOW_KEYS, "vault show (entsperrt)");
    assert_eq!(shown["state"], "unlocked");
    assert!(shown["shortening_threshold"].is_u64(), "{shown}");

    // check: a healthy vault has no entry; a file no name of the vault
    // explains is a finding, and a finding makes the exit code 1
    env.put(
        &local("ausgangsstand.txt", "Ausgangsstand\n"),
        &site,
        "/konflikt.txt",
    );
    env.put(&local("nur-a.txt", "Nur in A\n"), &site, "/nur-a.txt");
    let checked = env
        .se(&["vault", "check", site.as_str(), "--json"])
        .ok()
        .json();
    assert_keys(&checked, &["site", "items"], "vault check");
    assert_eq!(checked["site"], site.as_str());
    assert_eq!(checked["items"], json!([]), "{checked}");
    let junk = root_content_dir(&vault_dir).join(format!("{}.c9r", "A".repeat(48)));
    std::fs::write(&junk, b"fremd").expect("fremde Datei");
    let out = env.se(&["vault", "check", site.as_str(), "--json"]);
    assert_eq!(out.code, 1, "Befunde: Exitcode 1: {out}");
    let found = out.json();
    assert_keys(&found, &["site", "items"], "vault check (Befund)");
    assert_items(&found, "vault check: Eintrag");
    let items = found["items"].as_array().expect("items ist eine Liste");
    assert!(!items.is_empty(), "{found}");
    for item in items {
        assert_eq!(item["group"], "structure", "{item}");
        assert!(item["finding"].is_string(), "{item}");
    }
    assert!(
        items
            .iter()
            .any(|item| item["finding"] == "UndecryptableName"),
        "{found}"
    );
    std::fs::remove_file(junk).expect("fremde Datei entfernen");

    // sync add and the first run, which seeds the copy
    let added_job = env
        .se(&[
            "vault",
            "sync",
            "add",
            site.as_str(),
            "--with",
            "replica",
            "--name",
            "Arbeit-Kopie",
            "--trigger",
            "manual",
            "--json",
        ])
        .ok()
        .json();
    assert_keys(&added_job, &["job", "site_a", "site_b"], "vault sync add");
    assert_eq!(added_job["site_a"], site.as_str());
    let pair = Pair {
        job: field(&added_job, "job"),
        site_a: site.clone(),
        site_b: field(&added_job, "site_b"),
    };
    let seeded = env.seed_and_unlock_both(&pair);
    assert_keys(&seeded, &RUN_KEYS, "vault sync run");

    // a difference of each kind: new in A, new in B, changed on both sides
    env.put(
        &local("neu-a.txt", "Neu in A\n"),
        &pair.site_a,
        "/neu-a.txt",
    );
    env.put(
        &local("neu-b.txt", "Neu in B\n"),
        &pair.site_b,
        "/neu-b.txt",
    );
    env.change_both(
        &pair,
        "/konflikt.txt",
        &local(
            "konflikt-a.txt",
            "Fassung A, deutlich länger als der Ausgangsstand\n",
        ),
        &local("konflikt-b.txt", "Fassung B\n"),
    );
    settle();

    // compare
    let compared = env.compare(&pair.job);
    assert_keys(
        &compared,
        &["job", "sides", "revision", "counts", "items"],
        "vault compare",
    );
    assert_eq!(compared["job"], pair.job.as_str());
    assert_keys(&compared["sides"], &["a", "b"], "vault compare: sides");
    for (side, id) in [("a", &pair.site_a), ("b", &pair.site_b)] {
        assert_keys(
            &compared["sides"][side],
            &["site", "label"],
            "vault compare: sides",
        );
        assert_eq!(compared["sides"][side]["site"], id.as_str());
    }
    assert!(compared["revision"].is_u64(), "{compared}");
    assert_keys(&compared["counts"], &GROUP_KEYS, "vault compare: counts");
    assert_items(&compared, "vault compare: Eintrag");
    let conflict = item_at(&compared, "/konflikt.txt");
    assert_eq!(conflict["group"], "conflict", "{conflict}");
    assert_eq!(conflict["kind"], "file");
    assert_eq!(conflict["a"]["exists"], true);
    assert_eq!(conflict["b"]["exists"], true);
    let actions: Vec<&str> = conflict["actions"]
        .as_array()
        .expect("actions ist eine Liste")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(actions, ["keep-a", "keep-b", "keep-both", "merge"]);
    let new_a = item_at(&compared, "/neu-a.txt");
    assert_eq!(new_a["group"], "new_a", "{new_a}");
    assert_eq!(new_a["b"]["exists"], false);
    assert_eq!(new_a["actions"], json!(["transfer"]));
    assert_eq!(item_at(&compared, "/neu-b.txt")["group"], "new_b");

    // resolve: one entry, then every conflict
    let item = field(new_a, "item");
    let resolved = env
        .se(&[
            "vault",
            "resolve",
            pair.job.as_str(),
            item.as_str(),
            "--action",
            "transfer",
            "--json",
        ])
        .ok()
        .json();
    assert_keys(&resolved, &["item", "result"], "vault resolve");
    assert_eq!(resolved["item"], item.as_str());
    assert_eq!(resolved["result"], "ok");
    let all = env
        .se(&[
            "vault",
            "resolve",
            pair.job.as_str(),
            "--all",
            "--action",
            "keep-a",
            "--json",
        ])
        .ok()
        .json();
    assert_keys(&all, &["resolved", "failed"], "vault resolve --all");
    assert_eq!(all["failed"], json!([]), "{all}");
    assert!(all["resolved"].as_u64().unwrap_or(0) >= 1, "{all}");

    // after the resolutions: compare and run again
    let again = env.compare(&pair.job);
    assert_keys(
        &again,
        &["job", "sides", "revision", "counts", "items"],
        "vault compare (danach)",
    );
    assert_items(&again, "vault compare (danach): Eintrag");
    let last = env.sync_run(&pair.job);
    assert_keys(&last, &RUN_KEYS, "vault sync run (danach)");

    // list again: the vault is unlocked and names its job; remove
    let listed = env.se(&["vault", "list", "--json"]).ok().json();
    let entry = listed["vaults"]
        .as_array()
        .expect("vaults ist eine Liste")
        .iter()
        .flat_map(|group| group["sites"].as_array().expect("sites").iter())
        .find(|entry| entry["site"] == site.as_str())
        .unwrap_or_else(|| panic!("{site} fehlt in {listed}"));
    assert_keys(entry, &SITE_KEYS, "vault list (danach)");
    assert_eq!(entry["state"], "unlocked");
    assert_eq!(entry["sync_jobs"], json!([pair.job]));
    let removed = env.se(&["vault", "remove", copy_site.as_str()]).ok();
    assert_eq!(removed.stdout.trim(), format!("removed\t{copy_site}"));
    let listed = env.se(&["vault", "list", "--json"]).ok().json();
    assert!(!listed.to_string().contains(&copy_site), "{listed}");
}
