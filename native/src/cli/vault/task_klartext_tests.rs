//! Stages `vault_klartext` and `vault_klartext_drive` (vault plan VLT-M6 and
//! VLT-M14, C4): actions on the contents of an unlocked vault leave no
//! plaintext outside the vault. The fixture `marker` gives the isolated
//! folders of the test (`SE_VAULT_TASK_ROOT`) and a marker text
//! (`SE_VAULT_TASK_MARKER`). The tests put the marker into the names and the
//! contents of the vault and run the actions through the built `se` and its
//! background worker. The cleartext inputs and exports stay in the folder
//! `input`, which the search skips; the vault's own folders hold ciphertext
//! only. The test searches the root of the environment itself at the end, and
//! the suite checks `vault_task_klartext_marker_scan` and `…_drive` search it
//! again afterwards, together with the temporary folders of the machine.
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::task_env::{field, poll_until, text, vault_url, TaskEnv};
use super::task_flow::{item_at, settle};
use super::task_scan::{is_fuse_mount, marker_findings, marker_token};

/// Fails the test when the marker stands outside the folder `input`.
fn assert_no_marker(env: &TaskEnv, token: &str, extra_roots: &[&std::path::Path]) {
    let skip = [env.path("input")];
    let mut findings = marker_findings(env.root(), token, &skip);
    for root in extra_roots {
        findings.extend(marker_findings(root, token, &skip));
    }
    assert!(
        findings.is_empty(),
        "Klartext-Markierung außerhalb des Tresors ({}):\n{}",
        findings.len(),
        findings.join("\n")
    );
}

#[test]
#[ignore = "braucht fixture=marker und die gebaute se in SMART_EXPLORER_VAULT_SE_BIN (Stufe vault-klartext, env=SMART_EXPLORER_VAULT_SE_BIN={se})"]
fn vault_task_klartext_actions_leave_no_marker() {
    let env = TaskEnv::marker(false);
    let token = marker_token();
    // Local files carry the marker in their contents only: their names must
    // not, because logs of local paths are not about the vault.
    let local = |name: &str, content: &str| -> PathBuf {
        let path = env.path(&format!("input/{name}"));
        std::fs::write(&path, content).expect("lokale Datei");
        path
    };
    let content = |what: &str| format!("{token} {what}\nzweite Zeile {token}\n");

    let site = env.create_vault("orte/tresor-a", "Klartext-A");
    env.unlock(&site);
    let url = |path: &str| vault_url(&site, path);
    let folder = format!("/{token}-ordner");
    let file = format!("{folder}/{token}-datei.txt");
    let merge_file = format!("{folder}/{token}-merge.txt");

    // Copying in, reading, searching, copying inside the vault, renaming,
    // exporting, copying and deleting a folder.
    env.se(&["mkdir", url(&folder).as_str()]).ok();
    env.put(&local("quelle-a.txt", &content("Inhalt A")), &site, &file);
    assert_eq!(env.read_text(&site, &file), content("Inhalt A"));
    let hits = env.se(&["search", url("/").as_str(), token.as_str()]).ok();
    assert!(
        hits.stdout.contains(&format!("{token}-datei.txt")),
        "{hits}"
    );
    let copy = format!("{folder}/{token}-kopie.txt");
    env.se(&["cp", url(&file).as_str(), url(&copy).as_str()])
        .ok();
    let moved = format!("{folder}/{token}-verschoben.txt");
    env.se(&["mv", url(&copy).as_str(), url(&moved).as_str()])
        .ok();
    assert!(env.exists(&site, &moved) && !env.exists(&site, &copy));
    let export = env.path("input/ergebnis.txt");
    env.se(&["get", url(&moved).as_str(), text(&export)]).ok();
    assert_eq!(
        std::fs::read_to_string(export).expect("Export"),
        content("Inhalt A")
    );
    let folder_copy = format!("/{token}-ordner-2");
    let copied_file = format!("{folder_copy}/{token}-datei.txt");
    env.se(&[
        "cp",
        "-r",
        url(&folder).as_str(),
        url(&folder_copy).as_str(),
    ])
    .ok();
    assert!(env.exists(&site, &copied_file));
    env.se(&["rm", "-r", "-f", url(&folder_copy).as_str()]).ok();
    assert!(!env.exists(&site, &copied_file));

    // A file of about 4 MB: the copy of a larger file, in and out. It also
    // travels to the vault's copy in the first run of the job below.
    let big_text = format!("{token} Zeile\n").repeat(120_000);
    let big = format!("/{token}-gross.txt");
    env.put(&local("gross.txt", &big_text), &site, &big);
    let big_export = env.path("input/gross-ergebnis.txt");
    env.se(&["get", url(&big).as_str(), text(&big_export)]).ok();
    assert!(
        std::fs::read_to_string(big_export).expect("Export") == big_text,
        "die große Datei kommt verändert aus dem Tresor zurück"
    );

    // The vault job: the copy is seeded, then both sides change both files.
    let merge_start = format!("Zeile 1 {token}\nZeile 2\nZeile 3\n");
    env.put(&local("merge-start.txt", &merge_start), &site, &merge_file);
    let pair = env.sync_pair(&site);
    env.seed_and_unlock_both(&pair);
    env.change_both(
        &pair,
        &file,
        &local("quelle-b.txt", &content("Inhalt B, länger als der Anfang")),
        &local("quelle-c.txt", &content("C")),
    );
    let merge_a = format!("Zeile 1 A {token}\nZeile 2\nZeile 3\n");
    let merge_b = format!("Zeile 1 {token}\nZeile 2\nZeile 3 B, zweite Fassung\n");
    env.change_both(
        &pair,
        &merge_file,
        &local("merge-a.txt", &merge_a),
        &local("merge-b.txt", &merge_b),
    );
    settle();

    // The comparison lists both files with their cleartext names; both
    // versions of the first stay (keep both), the second becomes the text
    // the user merged (merge).
    let compared = env.compare(&pair.job);
    for path in [&file, &merge_file] {
        assert_eq!(item_at(&compared, path)["group"], "conflict", "{compared}");
    }
    let keep_item = field(item_at(&compared, &file), "item");
    let kept = env
        .se(&[
            "vault",
            "resolve",
            pair.job.as_str(),
            keep_item.as_str(),
            "--action",
            "keep-both",
            "--json",
        ])
        .ok()
        .json();
    assert_eq!(kept["result"], "ok", "{kept}");
    let listing = env.se(&["ls", url(&folder).as_str()]).ok();
    assert!(listing.stdout.contains("Konflikt"), "{listing}");
    let merge_item = field(item_at(&compared, &merge_file), "item");
    let merged_text = format!("Zeile 1 A {token}\nZeile 2\nZeile 3 B, zweite Fassung\n");
    let merged = local("zusammengefuehrt.txt", &merged_text);
    let result = env
        .se(&[
            "vault",
            "resolve",
            pair.job.as_str(),
            merge_item.as_str(),
            "--action",
            "merge",
            "--merged-file",
            text(&merged),
            "--json",
        ])
        .ok()
        .json();
    assert_eq!(result["result"], "ok", "{result}");
    assert_eq!(env.read_text(&pair.site_a, &merge_file), merged_text);
    assert_eq!(env.read_text(&pair.site_b, &merge_file), merged_text);
    let finished = env.sync_run(&pair.job);
    assert_eq!(finished["errors"], 0, "{finished}");

    // The worker dies at once (SIGKILL) while both vaults are unlocked, as in
    // a crash; nothing but the vault's ciphertext may be left behind.
    env.kill_processes();
    assert_no_marker(&env, &token, &[]);
}

/// Unmounts a FUSE mount that is still there when the test ends, for
/// instance after a failed assertion.
struct Unmount(PathBuf);

impl Drop for Unmount {
    fn drop(&mut self) {
        if is_fuse_mount(&self.0) {
            let _ = Command::new("fusermount3")
                .args(["-u", "-z"])
                .arg(&self.0)
                .status();
        }
    }
}

/// The folder where `se vault list` shows the drive of `site`.
fn mounted_path(listed: &Value, site: &str) -> Option<PathBuf> {
    for group in listed["vaults"].as_array()? {
        for entry in group["sites"].as_array()? {
            if entry["site"] == site {
                return entry["mounted"].as_str().map(PathBuf::from);
            }
        }
    }
    None
}

#[test]
#[ignore = "braucht fixture=marker,fuse,runtime-tmpfs und die gebaute se in SMART_EXPLORER_VAULT_SE_BIN (Stufe vault-klartext-drive, env=SMART_EXPLORER_VAULT_SE_BIN={se})"]
fn vault_task_klartext_drive_actions_leave_no_marker() {
    let env = TaskEnv::marker(true);
    let token = marker_token();
    let input = env.path("input");
    let source_text = format!("{token} Quelle\nzweite Zeile {token}\n");
    std::fs::write(input.join("quelle.txt"), &source_text).expect("Eingabe");

    // A vault with a file written through se before the drive exists.
    let site = env.create_vault("orte/tresor-laufwerk", "Klartext-Laufwerk");
    env.unlock(&site);
    let old_dir = format!("/{token}-alt");
    let old_file = format!("{old_dir}/{token}-vorher.txt");
    env.se(&["mkdir", vault_url(&site, &old_dir).as_str()]).ok();
    env.put(&input.join("quelle.txt"), &site, &old_file);

    // The drive: `se drive mount vault://<SITE>`; the vault list shows where
    // it is mounted, below the runtime folder.
    let target = format!("vault://{site}");
    env.se(&["drive", "mount", target.as_str()]).ok();
    let mut mount = None;
    let mounted = poll_until(Duration::from_secs(90), || {
        let listed = env.se(&["vault", "list", "--json"]).ok().json();
        mount = mounted_path(&listed, &site).filter(|path| is_fuse_mount(path));
        mount.is_some()
    });
    assert!(mounted, "das Tresor-Laufwerk erscheint nicht (mounted)");
    let mount = mount.expect("Einhängepfad");
    let _unmount = Unmount(mount.clone());
    assert!(
        mount.starts_with(env.runtime().join("smart-explorer").join("Tresore")),
        "Laufwerk liegt nicht unter dem Laufzeitordner: {}",
        mount.display()
    );

    // Actions through the drive: read what was there, create a folder and a
    // file, copy a local file in, rename, list, search, copy a file out,
    // delete.
    let old = mount
        .join(format!("{token}-alt"))
        .join(format!("{token}-vorher.txt"));
    assert_eq!(
        std::fs::read_to_string(old).expect("alte Datei"),
        source_text
    );
    let dir = mount.join(format!("{token}-ordner"));
    std::fs::create_dir(&dir).expect("Ordner im Laufwerk");
    let new_file = dir.join(format!("{token}-neu.txt"));
    let new_text = format!("{token} neu geschrieben\n");
    std::fs::write(&new_file, &new_text).expect("Datei im Laufwerk");
    let copied = dir.join(format!("{token}-kopie.txt"));
    std::fs::write(&copied, &source_text).expect("Kopie ins Laufwerk");
    let renamed = dir.join(format!("{token}-umbenannt.txt"));
    std::fs::rename(new_file, &renamed).expect("Umbenennen im Laufwerk");
    assert_eq!(
        std::fs::read_to_string(&renamed).expect("umbenannt"),
        new_text
    );
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("Ordner lesen")
        .map(|entry| {
            entry
                .expect("Eintrag")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            format!("{token}-kopie.txt"),
            format!("{token}-umbenannt.txt")
        ]
    );
    let hits = env.se(&["search", text(&mount), token.as_str()]).ok();
    assert!(
        hits.stdout.contains(&format!("{token}-umbenannt.txt")),
        "{hits}"
    );
    let export = std::fs::read(copied).expect("Datei aus dem Laufwerk");
    std::fs::write(input.join("export.txt"), export).expect("Export");
    std::fs::remove_file(renamed).expect("Löschen im Laufwerk");

    // The changes reach the vault.
    let copy_path = format!("/{token}-ordner/{token}-kopie.txt");
    let committed = poll_until(Duration::from_secs(90), || {
        let out = env.cat(&site, &copy_path);
        out.code == 0 && out.stdout == source_text
    });
    assert!(committed, "die Kopie erreicht den Tresor nicht");
    let renamed_path = format!("/{token}-ordner/{token}-umbenannt.txt");
    let gone = poll_until(Duration::from_secs(90), || {
        !env.exists(&site, &renamed_path)
    });
    assert!(gone, "die gelöschte Datei steht noch im Tresor");

    // Locking ends the drive.
    let mut last = String::new();
    let locked = poll_until(Duration::from_secs(60), || {
        let out = env.se(&["vault", "lock", "--all"]);
        last = out.to_string();
        out.code == 0
    });
    assert!(locked, "Sperren gescheitert: {last}");
    let unmounted = poll_until(Duration::from_secs(60), || !is_fuse_mount(&mount));
    if !unmounted {
        let _ = Command::new("fusermount3")
            .args(["-u", "-z"])
            .arg(mount.as_path())
            .status();
    }
    assert!(unmounted, "Sperren hängt das Laufwerk nicht aus");

    // Nothing but the vault's ciphertext is left behind, neither below the
    // root of the environment nor in the runtime folder.
    env.kill_processes();
    assert_no_marker(&env, &token, &[env.runtime()]);
}
