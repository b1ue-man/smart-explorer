//! Pure coordination for changes spanning secure storage and metadata.

/// Apply a secure-store mutation and then commit its matching metadata.
///
/// The caller supplies a rollback built from a previously captured snapshot.
/// Credential-store mutations are treated as atomic: a reported mutation
/// failure prevents the metadata commit, while a later metadata failure rolls
/// the successful credential change back.
pub(super) fn commit_secret_and_metadata(
    action: &str,
    mutate_secret: impl FnOnce() -> Result<(), String>,
    commit_metadata: impl FnOnce() -> Result<(), String>,
    rollback_secret: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    mutate_secret().map_err(|error| {
        format!("{action}: Anmeldeinformationen konnten nicht geändert werden: {error}")
    })?;
    match commit_metadata() {
        Ok(()) => Ok(()),
        Err(commit_error) => rollback_error(action, commit_error, rollback_secret),
    }
}

fn rollback_error(
    action: &str,
    error: String,
    rollback_secret: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match rollback_secret() {
        Ok(()) => Err(format!(
            "{action}: Metadaten konnten nicht gespeichert werden: {error}; vorheriger Zustand wiederhergestellt"
        )),
        Err(rollback_error) => Err(format!(
            "{action}: Metadaten konnten nicht gespeichert werden: {error}; Wiederherstellung der Anmeldeinformationen fehlgeschlagen: {rollback_error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn secret_failure_skips_metadata_and_rollback() {
        let metadata_called = Cell::new(false);
        let rollback_called = Cell::new(false);

        let error = commit_secret_and_metadata(
            "Speichern",
            || Err("store locked".into()),
            || {
                metadata_called.set(true);
                Ok(())
            },
            || {
                rollback_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(!metadata_called.get());
        assert!(!rollback_called.get());
        assert!(error.contains("store locked"));
    }

    #[test]
    fn metadata_failure_rolls_back_secret() {
        let secret = Cell::new("old");

        let error = commit_secret_and_metadata(
            "Speichern",
            || {
                secret.set("new");
                Ok(())
            },
            || Err("disk full".into()),
            || {
                secret.set("old");
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(secret.get(), "old");
        assert!(error.contains("disk full"));
        assert!(error.contains("wiederhergestellt"));
    }

    #[test]
    fn rollback_failure_reports_partial_state() {
        let error = commit_secret_and_metadata(
            "Entfernen",
            || Ok(()),
            || Err("read-only file".into()),
            || Err("credential store locked".into()),
        )
        .unwrap_err();

        assert!(error.contains("read-only file"));
        assert!(error.contains("Wiederherstellung"));
        assert!(error.contains("credential store locked"));
    }
}
