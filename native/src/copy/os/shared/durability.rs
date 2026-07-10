use super::move_guard::{remove_quarantine, QuarantinedSource};
use std::io;
use std::path::Path;

pub(super) fn finish_direct_move<F>(
    target: &Path,
    source: &QuarantinedSource,
    mut sync_parent: F,
) -> io::Result<()>
where
    F: FnMut(&Path) -> io::Result<()>,
{
    sync_parent(target).map_err(|error| {
        committed_error(
            &error,
            format!(
                "move committed at {}; source is no longer at {}",
                target.display(),
                source.original.display()
            ),
            "destination parent durability sync failed",
        )
    })?;
    if source.original.parent() != target.parent() {
        sync_parent(&source.original).map_err(|error| {
            committed_error(
                &error,
                format!(
                    "move committed at {}; source is no longer at {}",
                    target.display(),
                    source.original.display()
                ),
                "source parent durability sync failed",
            )
        })?;
    }
    Ok(())
}

pub(super) fn finish_staged_commit<F>(
    target: &Path,
    original_source: &Path,
    quarantine: &mut Option<QuarantinedSource>,
    mut sync_parent: F,
) -> io::Result<()>
where
    F: FnMut(&Path) -> io::Result<()>,
{
    sync_parent(target).map_err(|error| {
        let state = match quarantine.as_ref() {
            Some(source) => format!(
                "destination committed at {}; secured move source remains at {}; original source is absent at {}",
                target.display(),
                source.path.display(),
                source.original.display()
            ),
            None => format!(
                "copy destination committed at {}; source remains at {}",
                target.display(),
                original_source.display()
            ),
        };
        committed_error(&error, state, "destination parent durability sync failed")
    })?;

    let Some(source) = quarantine.take() else {
        return Ok(());
    };
    remove_quarantine(&source).map_err(|error| {
        committed_error(
            &error,
            format!(
                "destination committed at {}; secured move source remains at {}",
                target.display(),
                source.path.display()
            ),
            "secured source cleanup failed",
        )
    })?;
    sync_parent(&source.original).map_err(|error| {
        committed_error(
            &error,
            format!(
                "move destination committed at {}; secured source was removed and original source is absent at {}",
                target.display(),
                source.original.display()
            ),
            "source parent durability sync failed",
        )
    })
}

fn committed_error(error: &io::Error, state: String, operation: &str) -> io::Error {
    io::Error::new(error.kind(), format!("{state}; {operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copy::platform;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "se-durability-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn injected() -> io::Error {
        io::Error::other("injected sync failure")
    }

    #[test]
    fn copy_parent_sync_failure_reports_that_destination_is_committed() {
        let root = root("copy");
        let source = root.join("source");
        let target = root.join("target");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&target, b"committed").unwrap();
        let mut quarantine = None;
        let error = finish_staged_commit(&target, &source, &mut quarantine, |_| Err(injected()))
            .unwrap_err();
        assert!(error.to_string().contains("copy destination committed"));
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        assert_eq!(std::fs::read(&target).unwrap(), b"committed");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn move_parent_sync_failure_reports_and_retains_quarantine() {
        let root = root("move-retained");
        let source_path = root.join("source");
        let target = root.join("target");
        std::fs::write(&source_path, b"source").unwrap();
        std::fs::write(&target, b"committed").unwrap();
        let mut source = Some(super::super::move_guard::quarantine_source(&source_path).unwrap());
        let secured = source.as_ref().unwrap().path.clone();
        let error = finish_staged_commit(&target, &source_path, &mut source, |_| Err(injected()))
            .unwrap_err();
        assert!(error
            .to_string()
            .contains(secured.to_string_lossy().as_ref()));
        assert!(source.is_some());
        assert!(secured.exists());
        assert!(!source_path.exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn source_parent_sync_failure_reports_post_cleanup_state() {
        let root = root("move-cleaned");
        let source_path = root.join("source");
        let target = root.join("target");
        std::fs::write(&source_path, b"source").unwrap();
        std::fs::write(&target, b"committed").unwrap();
        let mut source = Some(super::super::move_guard::quarantine_source(&source_path).unwrap());
        let secured = source.as_ref().unwrap().path.clone();
        let calls = AtomicUsize::new(0);
        let error = finish_staged_commit(&target, &source_path, &mut source, |_| {
            if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                Ok(())
            } else {
                Err(injected())
            }
        })
        .unwrap_err();
        assert!(error.to_string().contains("secured source was removed"));
        assert!(source.is_none());
        assert!(!secured.exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn direct_move_sync_failure_reports_committed_target() {
        let root = root("direct");
        let source_path = root.join("source");
        let target = root.join("target");
        std::fs::write(&source_path, b"source").unwrap();
        let source = super::super::move_guard::quarantine_source(&source_path).unwrap();
        platform::move_file(&source.path, &target, false).unwrap();
        let error = finish_direct_move(&target, &source, |_| Err(injected())).unwrap_err();
        assert!(error.to_string().contains("move committed"));
        assert!(target.exists());
        assert!(!source_path.exists());
        std::fs::remove_dir_all(root).ok();
    }
}
