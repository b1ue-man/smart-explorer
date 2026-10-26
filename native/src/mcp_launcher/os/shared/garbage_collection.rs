//! Collect only proved immutable image bytes; retain every unowned child.
use crate::mcp_launcher::{os, Installation};
use std::path::Path;

pub fn garbage_collect(root: &Path) -> Result<Vec<String>, String> {
    let installation = Installation::load(root)?;
    let _owner = installation.lock("update-owner.lock", true)?;
    let _control = installation.lock("control.lock", true)?;
    let state = installation.state()?;
    if state.as_ref().is_some_and(|s| s.blocks_work()) {
        return Err("update_recovery_required".into());
    }
    let mut removed = Vec::new();
    let runtimes = root.join("mcp/runtimes");
    os::validate_tree(root, &runtimes)?;
    for entry in std::fs::read_dir(&runtimes).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue; };
        let Some((hash, retired)) = generation(&name) else { continue; };
        if hash == installation.manifest.runtime_sha256
            || state.as_ref().is_some_and(|s| s.previous.runtime_sha256 == hash) {
            continue;
        }
        let path = entry.path();
        os::validate_tree(root, &path)?;
        // Do not move a user's files away from their original hashpath.
        if !retired && has_foreign_children(&path)? {
            continue;
        }
        let image = path.join(os::CLI_NAME);
        let lifetime = path.join("lifetime.lock");
        let image_exists = exists(&image)?;
        let lifetime_exists = exists(&lifetime)?;
        if (!retired || image_exists) && !lifetime_exists {
            continue; // an extant program without its lifetime proof is not idle
        }
        let lifetime_guard = if lifetime_exists {
            os::validate_tree(root, &lifetime)?;
            if !std::fs::symlink_metadata(&lifetime).map_err(|e| e.to_string())?.is_file()
                || !std::fs::read(&lifetime).map_err(|e| e.to_string())?.is_empty() {
                continue;
            }
            let Ok(lock) = os::lock(&lifetime, true) else { continue; };
            Some(lock)
        } else { None };
        let image_guard = if image_exists {
            let Ok(verified) = os::open_verified(root, &image, hash) else { continue; };
            let Ok(guard) = os::removal_image_guard(&image) else { continue; };
            match os::processes_for(&image) {
                Ok(processes) if processes.is_empty() => {}
                _ => continue,
            }
            drop(verified);
            guard
        } else if !retired { continue; } else { None };
        let retired_path = if retired { path } else {
            let target = runtimes.join(format!(".retired-{hash}-{}", crate::mcp_launcher::provision::unique_id()?));
            if os::rename_durable(&path, &target).is_err() { continue; }
            target
        };
        // Keep exclusion until the last executable is gone. An interrupted
        // quarantine remains discoverable by the next call to this function.
        let image = retired_path.join(os::CLI_NAME);
        if image_exists {
            let verified = os::open_verified(root, &image, hash)?;
            drop(verified);
            std::fs::remove_file(&image).map_err(|e| e.to_string())?;
        }
        drop(image_guard);
        drop(lifetime_guard);
        let lifetime = retired_path.join("lifetime.lock");
        if lifetime_exists {
            os::validate_tree(root, &lifetime)?;
            if !std::fs::symlink_metadata(&lifetime).map_err(|e| e.to_string())?.is_file()
                || !std::fs::read(&lifetime).map_err(|e| e.to_string())?.is_empty() {
                return Err("runtime_gc_lifetime_changed: quarantine retained".into());
            }
            std::fs::remove_file(&lifetime).map_err(|e| e.to_string())?;
        }
        match std::fs::remove_dir(&retired_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(e) => return Err(e.to_string()),
        }
        if image_exists { removed.push(hash.to_owned()); }
    }
    Ok(removed)
}

fn generation(name: &str) -> Option<(&str, bool)> {
    use crate::mcp_launcher::core::manifest::{is_id, is_sha256};
    if is_sha256(name) { return Some((name, false)); }
    let (hash, id) = name.strip_prefix(".retired-")?.split_once('-')?;
    (is_sha256(hash) && is_id(id)).then_some((hash, true))
}

fn has_foreign_children(path: &Path) -> Result<bool, String> {
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_name().to_str().is_some_and(|name| name == os::CLI_NAME || name == "lifetime.lock") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;
    use super::super::batches_lifecycle_task_tests::Fixture;
    fn old(fixture: &Fixture) -> (String, std::path::PathBuf) {
        let bytes = b"\x7fELF-retired-fixture";
        let hash = crate::mcp_launcher::installation::hash_bytes(bytes);
        let path = fixture.installation.root.join("mcp/runtimes").join(&hash);
        std::fs::create_dir(&path).unwrap();
        os::protect(&path, true).unwrap();
        std::fs::write(path.join(os::CLI_NAME), bytes).unwrap();
        os::protect(&path.join(os::CLI_NAME), true).unwrap();
        std::fs::write(path.join("lifetime.lock"), b"").unwrap();
        (hash, path)
    }

    #[test]
    fn batches_p13_gc_preserves_foreign_children_at_their_original_paths() {
        let fixture = Fixture::new();
        let (_, path) = old(&fixture);
        let user = path.join("user-notes.txt");
        std::fs::write(&user, b"keep").unwrap();
        let nested = path.join("user-directory");
        std::fs::create_dir(&nested).unwrap();
        assert!(garbage_collect(&fixture.installation.root).unwrap().is_empty());
        assert_eq!(std::fs::read(&user).unwrap(), b"keep");
        assert!(nested.is_dir());
        assert!(path.join(os::CLI_NAME).exists());
        assert!(fixture.installation.runtime_path().exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batches_p13_gc_collects_only_proved_idle_images_and_retries_quarantines() {
        let fixture = Fixture::new();
        let (hash, path) = old(&fixture);
        let held = os::lock(&path.join("lifetime.lock"), false).unwrap();
        assert!(garbage_collect(&fixture.installation.root).unwrap().is_empty());
        drop(held);
        let retired = path.with_file_name(format!(".retired-{hash}-interrupted"));
        std::fs::rename(&path, &retired).unwrap();
        let user = retired.join("user-link");
        let foreign = tempfile::tempdir().unwrap();
        std::fs::write(foreign.path().join("keep.txt"), b"foreign").unwrap();
        os::create_test_directory_link(foreign.path(), &user).unwrap();
        assert_eq!(garbage_collect(&fixture.installation.root).unwrap(), [hash]);
        assert!(!retired.join(os::CLI_NAME).exists());
        assert!(!retired.join("lifetime.lock").exists());
        assert!(user.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(foreign.path().join("keep.txt")).unwrap(), b"foreign");
        assert!(garbage_collect(&fixture.installation.root).unwrap().is_empty());
        assert!(user.symlink_metadata().unwrap().file_type().is_symlink());
    }

    #[test]
    fn batches_p13_gc_changed_images_or_locks_never_authorize_deletion() {
        for changed in [os::CLI_NAME, "lifetime.lock"] {
            let fixture = Fixture::new();
            let (_, path) = old(&fixture);
            std::fs::write(path.join(changed), b"changed user content").unwrap();
            assert!(garbage_collect(&fixture.installation.root).unwrap().is_empty());
            assert_eq!(std::fs::read(path.join(changed)).unwrap(), b"changed user content");
            assert!(path.join(os::CLI_NAME).exists());
        }
    }
}
