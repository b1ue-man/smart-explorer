//! K44 of the vault plan: `fs.fetch`, `fs.open` and `fs.materialize` never
//! put a cleartext copy of a vault file below the app cache; the app opens
//! vault files through the document provider (`vault_use_provider`).
use super::super::config::HostSettings;
use super::super::dispatch::dispatch;
use super::super::runtime::Runtime;
use serde_json::json;
use std::path::Path;

/// Every file name below `dir`.
fn names(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(entry.file_name().to_string_lossy().into_owned());
            if path.is_dir() {
                out.extend(names(&path));
            }
        }
    }
    out
}

#[test]
fn vault_task_android_fetch_rejects_vault() {
    let dir = tempfile::tempdir().expect("temp dir");
    let files = dir.path().join("files");
    let cache = dir.path().join("cache");
    for path in [&files, &cache] {
        std::fs::create_dir_all(path).expect("fixture dir");
    }
    let config = json!({ "filesDir": files, "cacheDir": cache, "startDaemon": false });
    let settings = HostSettings::parse(&config.to_string()).expect("settings");
    let rt = Runtime::detached(settings);
    let site = "0123456789abcdef0123456789abcdef";
    for location in [
        format!("vault://{site}/Geheim/Plan.txt"),
        "vault://Mein Tresor/Plan.txt".to_string(),
    ] {
        for (method, args) in [
            ("fs.fetch", json!({ "location": location })),
            ("fs.open", json!({ "location": location })),
            ("fs.materialize", json!({ "locations": [location] })),
        ] {
            let error = dispatch(&rt, method, &args).expect_err(method);
            assert_eq!(
                error.kind,
                crate::vault::USE_PROVIDER_CODE,
                "{method} {location}"
            );
        }
    }
    let cached = names(&cache);
    assert!(
        !cached.iter().any(|name| name.contains("Plan")),
        "no copy below the cache: {cached:?}"
    );
    // A local file keeps opening directly, as before.
    let local = files.join("frei.txt");
    std::fs::write(&local, b"frei").expect("local file");
    let opened = dispatch(
        &rt,
        "fs.open",
        &json!({ "location": local.to_string_lossy() }),
    )
    .expect("local open");
    assert_eq!(opened["localPath"], local.to_string_lossy().as_ref());
}
