//! K13/K14 of the vault plan: favourites, recent places and folder sort
//! settings inside a vault are stored globally only as the vault root; the
//! places inside are sealed per vault and come back only while it is
//! unlocked. The desktop files (`favorites.txt`, `recent.txt`,
//! `dir_sort.tsv`) and the Android list (`mobile/recent.json`) use these
//! functions; vault drives and opened vault backends are known by place.
use std::collections::HashMap;
use std::sync::Arc;

use super::*;
use crate::vault::core::lock_policy::{LockPlatform, LockReason};
use crate::vault::core::registry_types::LockSettings;

use super::super::session::{LockRequest, SessionRegister};
use super::super::session_test_support::{test_register, unlocked_site};

/// Sealing with a register of the test (no worker, no process table).
struct RegisterSealing(Arc<SessionRegister>);

impl SiteSealing for RegisterSealing {
    fn seal(
        &self,
        site: &str,
        purpose: SealPurpose,
        items: &[&str],
    ) -> Result<Vec<String>, String> {
        self.0
            .seal_texts(site, purpose, items)
            .map_err(|error| error.to_string())
    }

    fn unseal(
        &self,
        site: &str,
        purpose: SealPurpose,
        items: &[String],
    ) -> Result<Vec<String>, String> {
        self.0
            .unseal_texts(site, purpose, items)
            .map(|texts| texts.iter().map(|text| text.as_str().to_string()).collect())
            .map_err(|error| error.to_string())
    }
}

const MARKER: &str = "Geheimprojekt7c2";

fn contains(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn vault_task_favorites_and_recent_inside_vault_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let register = test_register(LockPlatform::Desktop);
    let (site, _session) = unlocked_site(&register, LockSettings::desktop());
    let sealing = RegisterSealing(Arc::clone(&register));
    let store = LocalStore::at(dir.path().join("vaults"));
    let root = format!("vault://{site}/");
    let inner = format!("vault://{site}/{MARKER}/Plan.txt");
    let deeper = format!("vault://{site}/Akten/{MARKER}");

    // Desktop favourites (`favorites.txt`): the vault root once, where its
    // first place stood; other places unchanged and in order.
    let favorites = vec![
        inner.clone(),
        "/home/anna/Dokumente".to_string(),
        root.clone(),
        deeper.clone(),
    ];
    let global = store_places(&store, &sealing, PlaceList::Favorites, &favorites);
    assert_eq!(global, [root.clone(), "/home/anna/Dokumente".to_string()]);
    let favorites_file = dir.path().join("favorites.txt");
    std::fs::write(&favorites_file, global.join("\n")).unwrap();
    assert!(!contains(&std::fs::read(&favorites_file).unwrap(), MARKER));
    let sealed = std::fs::read(store.path(&site)).expect("sealed place data");
    assert!(!contains(&sealed, MARKER), "no cleartext in <site>.local");
    assert!(!contains(&sealed, "Akten"));
    // Unlocked: the places inside come back in place of the root.
    let shown = expand_places(&store, &sealing, PlaceList::Favorites, &global);
    assert_eq!(
        shown,
        [
            inner.clone(),
            root.clone(),
            deeper.clone(),
            "/home/anna/Dokumente".to_string()
        ]
    );

    // Recent places: desktop `recent.txt` and Android `mobile/recent.json`.
    let recent = vec![deeper.clone(), "sftp://u@nas:22/fotos".to_string()];
    let global_recent = store_places(&store, &sealing, PlaceList::Recent, &recent);
    assert_eq!(
        global_recent,
        [root.clone(), "sftp://u@nas:22/fotos".to_string()]
    );
    let json = serde_json::to_string(&global_recent).unwrap();
    assert!(!json.contains(MARKER), "recent.json: {json}");
    let back: Vec<String> = serde_json::from_str(&json).unwrap();
    assert_eq!(
        expand_places(&store, &sealing, PlaceList::Recent, &back),
        recent
    );

    // Folder sort settings (`dir_sort.tsv`).
    let sort: HashMap<String, bool> = [
        (format!("vault://{site}/{MARKER}"), true),
        (root.clone(), false),
        ("/home/anna".to_string(), true),
    ]
    .into_iter()
    .collect();
    let global_sort = store_dir_sort(&store, &sealing, &sort);
    assert_eq!(global_sort.len(), 2);
    assert!(global_sort.keys().all(|key| !key.contains(MARKER)));
    assert_eq!(expand_dir_sort(&store, &sealing, global_sort.clone()), sort);

    // Locked: only the roots remain visible, and nothing new is written in
    // cleartext; the sealed data stays as it was.
    let before = std::fs::read(store.path(&site)).unwrap();
    register
        .lock(&site, LockRequest::new(LockReason::Explicit, true))
        .expect("locked");
    assert_eq!(
        expand_places(&store, &sealing, PlaceList::Favorites, &global),
        global
    );
    assert_eq!(
        expand_dir_sort(&store, &sealing, global_sort.clone()),
        global_sort
    );
    let more = vec![format!("vault://{site}/Neu-{MARKER}")];
    assert_eq!(
        store_places(&store, &sealing, PlaceList::Favorites, &more),
        [root]
    );
    assert_eq!(std::fs::read(store.path(&site)).unwrap(), before);
    for entry in std::fs::read_dir(dir.path().join("vaults")).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(!contains(&bytes, MARKER));
    }

    // Places of vault drives and opened vault backends are known.
    let drive = dir.path().join("drive");
    publish_vault_mount(&site, &drive);
    assert_eq!(vault_mount_of(&drive.join("x")), Some(Some(site.clone())));
    assert!(shows_vault(&drive.join("x").to_string_lossy()));
    assert!(shows_vault(&format!("vault://{site}/x")));
    withdraw_vault_mount(&site, &drive);
    assert!(!shows_vault(&drive.join("x").to_string_lossy()));
    assert!(!shows_vault("/home/anna/Dokumente"));
}
