//! Stage `vault_cli` (vault plan VLT-M9, C7.3, U26-10 to U26-12), through the
//! built `se` in an isolated environment: the shell completion of `se vault`
//! as the shell asks for it (`COMPLETE=fish se -- se vault …`), and the words
//! of a call that meets a locked vault. The key sets of the JSON are in
//! `task_json_tests.rs`.
use super::task_env::{run, vault_url, TaskEnv};
use crate::vault::{CompareGroup, ItemAction};

/// The candidates `se` offers for `words` (the command line up to the word
/// to complete, which is empty for a new word): value and help text.
fn complete(env: &TaskEnv, words: &[&str]) -> Vec<(String, String)> {
    let mut command = env.command(&["--"]);
    command.args(words).env("COMPLETE", "fish");
    let out = run(
        command,
        None,
        format!("COMPLETE=fish se -- {}", words.join(" ")),
    )
    .ok();
    out.stdout
        .lines()
        .map(|line| match line.split_once('\t') {
            Some((value, help)) => (value.to_string(), help.to_string()),
            None => (line.to_string(), String::new()),
        })
        .collect()
}

fn sorted_values(candidates: &[(String, String)]) -> Vec<&str> {
    let mut values: Vec<&str> = candidates.iter().map(|(value, _)| value.as_str()).collect();
    values.sort_unstable();
    values
}

#[test]
#[ignore = "braucht die gebaute se in SMART_EXPLORER_VAULT_SE_BIN (Stufe vault-cli, env=SMART_EXPLORER_VAULT_SE_BIN={se})"]
fn vault_task_cli_completion() {
    let env = TaskEnv::isolated();
    let site = env.create_vault("orte/vervollstaendigung", "Vervollstaendigung");
    let pair = env.sync_pair(&site);
    let job_name = "Tresor Vervollstaendigung";

    // `COMPLETE=bash se` prints the script a shell sources; it names `se`.
    let mut command = env.command(&[]);
    command.env("COMPLETE", "bash");
    let script = run(command, None, "COMPLETE=bash se".to_string()).ok();
    assert!(script.stdout.contains("_clap_complete_se"), "{script}");

    // Every command of `se vault`, each with its help text.
    let commands = [
        "list",
        "show",
        "detect",
        "add",
        "create",
        "remove",
        "unlock",
        "lock",
        "password",
        "forget-password",
        "settings",
        "check",
        "sync",
        "compare",
        "resolve",
    ];
    let offered = complete(&env, &["se", "vault", ""]);
    for command in commands {
        let entry = offered.iter().find(|(value, _)| value == command);
        match entry {
            Some((_, help)) => assert!(!help.is_empty(), "{command} ohne Hilfetext"),
            None => panic!("{command} fehlt in {offered:?}"),
        }
    }
    let sync = complete(&env, &["se", "vault", "sync", ""]);
    for command in ["add", "run"] {
        assert!(
            sync.iter().any(|(value, _)| value == command),
            "sync {command} fehlt in {sync:?}"
        );
    }

    // The vaults of the vault list: ID, with the label as help text.
    let entry = (site.clone(), "Vervollstaendigung".to_string());
    for command in [
        "show",
        "remove",
        "unlock",
        "lock",
        "password",
        "forget-password",
        "settings",
        "check",
    ] {
        let offered = complete(&env, &["se", "vault", command, ""]);
        assert!(offered.contains(&entry), "{command}: {offered:?}");
        assert!(
            !offered.iter().any(|(value, _)| *value == pair.job),
            "{command} bietet einen Job an: {offered:?}"
        );
    }
    let offered = complete(&env, &["se", "vault", "sync", "add", ""]);
    assert!(offered.contains(&entry), "sync add: {offered:?}");
    // The copy the job keeps is a vault of the list as well.
    assert!(
        offered.iter().any(|(value, _)| *value == pair.site_b),
        "die Kopie fehlt: {offered:?}"
    );

    // The vault jobs: ID, with the name of the job as help text.
    let job = (pair.job.clone(), job_name.to_string());
    let job_words: [&[&str]; 3] = [
        &["se", "vault", "compare", ""],
        &["se", "vault", "resolve", ""],
        &["se", "vault", "sync", "run", ""],
    ];
    for words in job_words {
        let offered = complete(&env, words);
        assert!(offered.contains(&job), "{words:?}: {offered:?}");
        assert!(
            !offered.iter().any(|(value, _)| *value == site),
            "{words:?} bietet einen Tresor an: {offered:?}"
        );
    }

    // The values of the options: the nine groups and `all`, the 19 actions.
    let groups = complete(
        &env,
        &["se", "vault", "compare", pair.job.as_str(), "--group", ""],
    );
    let mut expected: Vec<&str> = CompareGroup::ALL.iter().map(|group| group.key()).collect();
    expected.push("all");
    expected.sort_unstable();
    assert_eq!(expected.len(), 10);
    assert_eq!(sorted_values(&groups), expected, "--group");
    let actions = complete(
        &env,
        &[
            "se",
            "vault",
            "resolve",
            pair.job.as_str(),
            "0123456789abcdef",
            "--action",
            "",
        ],
    );
    let mut expected: Vec<&str> = ItemAction::KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(expected.len(), 19);
    assert_eq!(sorted_values(&actions), expected, "--action");
}

/// Asserts that `se <args>` failed with exit code 1 and names the call that
/// unlocks the vault.
fn assert_names_unlock(env: &TaskEnv, args: &[&str], unlock: &str) {
    let out = env.se(args).failed();
    assert!(
        out.stderr.contains(unlock),
        "die Meldung nennt `{unlock}` nicht: {out}"
    );
}

#[test]
#[ignore = "braucht die gebaute se in SMART_EXPLORER_VAULT_SE_BIN (Stufe vault-cli, env=SMART_EXPLORER_VAULT_SE_BIN={se})"]
fn vault_task_cli_locked_vault_names_unlock() {
    let env = TaskEnv::isolated();
    let site = env.create_vault("orte/gesperrt", "Gesperrt");
    let pair = env.sync_pair(&site);
    let unlock = format!("se vault unlock {site}");
    let file = vault_url(&site, "/datei.txt");
    let root = vault_url(&site, "/");
    let calls: [&[&str]; 5] = [
        &["cat", file.as_str()],
        &["ls", root.as_str()],
        &["stat", file.as_str()],
        &["vault", "check", site.as_str()],
        &["vault", "compare", pair.job.as_str()],
    ];

    // A vault that was never unlocked.
    for call in calls {
        assert_names_unlock(&env, call, &unlock);
    }
    // The sync of a vault job needs no key: it runs although the vault is
    // locked, so only the calls that need the key name the unlock.
    let seeded = env.sync_run(&pair.job);
    assert_eq!(seeded["errors"], 0, "{seeded}");

    // A vault that was unlocked and locked again.
    env.unlock(&site);
    let locked = env.lock_all();
    assert!(locked.stdout.contains(&site), "{locked}");
    for call in calls {
        assert_names_unlock(&env, call, &unlock);
    }
    // Written as the label, the vault is still named by its ID.
    assert_names_unlock(&env, &["cat", "vault://Gesperrt/datei.txt"], &unlock);
}
