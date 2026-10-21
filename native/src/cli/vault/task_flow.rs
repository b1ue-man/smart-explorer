//! The vault sync scenario the stage tests share (vault plan C5, C6, C7.3):
//! a vault job between a vault and its copy, the first run that seeds the
//! copy, two files that both sides change (a conflict), and the comparison
//! that lists the differences. Everything goes through the built `se`, so
//! the commands, the background worker and the exit codes are the real ones.
use std::path::Path;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::task_env::{field, vault_url, TaskEnv};

/// A vault job and the two sites it keeps alike.
pub(crate) struct Pair {
    pub(crate) job: String,
    pub(crate) site_a: String,
    pub(crate) site_b: String,
}

/// Lets the files written so far age a little, so that no file counts as
/// just written when the job plans its run.
pub(crate) fn settle() {
    thread::sleep(Duration::from_secs(2));
}

/// The entry of a comparison whose cleartext path is `path`.
pub(crate) fn item_at<'a>(compare: &'a Value, path: &str) -> &'a Value {
    compare["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["path"] == path))
        .unwrap_or_else(|| panic!("kein Eintrag für {path} im Vergleich: {compare}"))
}

impl TaskEnv {
    /// `se vault sync add` of a vault job between `site` and a new copy that
    /// `se` keeps (started by hand only); the copy still waits to be seeded.
    pub(crate) fn sync_pair(&self, site: &str) -> Pair {
        let added = self
            .se(&[
                "vault",
                "sync",
                "add",
                site,
                "--with",
                "replica",
                "--trigger",
                "manual",
                "--json",
            ])
            .ok()
            .json();
        Pair {
            job: field(&added, "job"),
            site_a: field(&added, "site_a"),
            site_b: field(&added, "site_b"),
        }
    }

    /// `se vault sync run` of the job `job`; the JSON it printed.
    pub(crate) fn sync_run(&self, job: &str) -> Value {
        self.se(&["vault", "sync", "run", job, "--json"])
            .ok()
            .json()
    }

    /// `se vault compare` of the job `job` for the default groups; the JSON
    /// it printed.
    pub(crate) fn compare(&self, job: &str) -> Value {
        self.se(&["vault", "compare", job, "--json"]).ok().json()
    }

    /// The first run of a new job: the copy is seeded from `pair.site_a`,
    /// then both sites are unlocked so that files can be written to both.
    pub(crate) fn seed_and_unlock_both(&self, pair: &Pair) -> Value {
        settle();
        let seeded = self.sync_run(&pair.job);
        assert_eq!(seeded["errors"], 0, "Lauf zum Befüllen der Kopie: {seeded}");
        assert!(
            seeded["copied_a_to_b"].as_u64().unwrap_or(0) > 0,
            "die Kopie bekam nichts: {seeded}"
        );
        self.unlock(&pair.site_a);
        self.unlock(&pair.site_b);
        seeded
    }

    /// Writes `local_a` to `path` in the site A and `local_b` to `path` in
    /// the site B of `pair`: after a run that both sides share, this is a
    /// conflict, because both sides changed the file.
    pub(crate) fn change_both(&self, pair: &Pair, path: &str, local_a: &Path, local_b: &Path) {
        self.put(local_a, &pair.site_a, path);
        self.put(local_b, &pair.site_b, path);
    }

    /// The cleartext of `path` in `site`, as text.
    pub(crate) fn read_text(&self, site: &str, path: &str) -> String {
        self.cat(site, path).ok().stdout
    }

    /// Whether `path` in the vault `site` exists (`se cat` succeeds).
    pub(crate) fn exists(&self, site: &str, path: &str) -> bool {
        self.se(&["cat", vault_url(site, path).as_str()]).code == 0
    }
}
