//! Windows virtual descriptors have a fixed MAX_PATH-sized relative name.
//! Longer trees use the same exact file snapshot in a temporary directory.
use super::clipboard_lifecycle::PreparedTempClipboard;
use super::clipboard_state::{PreparationResult, PreparationStamp};
use super::prelude::*;
use super::*;
use crate::app::shared_platform_helpers::ClipboardVirtualFile;

impl App {
    pub(in crate::app) fn materialize_long_clipboard(
        &mut self,
        files: Vec<ClipboardVirtualFile>,
        stamp: PreparationStamp,
    ) {
        // K05 (3) of the vault plan: files of a vault drive are never copied
        // to a cleartext folder for the clipboard.
        if materialize_refused(&files) {
            self.cancel_clipboard_preparation();
            self.error_msg = Some(crate::vault::PROVIDE_REFUSED.to_string());
            return;
        }
        let (tx, rx) = unbounded();
        let worker = std::thread::Builder::new()
            .name("clip-long-paths".into())
            .spawn(move || {
                let backend = crate::vfs::LocalBackend::new("");
                let result =
                    download_clipboard_snapshot(&backend, files).map(PreparedTempClipboard::new);
                let _ = tx.send(PreparationResult { stamp, result });
            });
        match worker {
            Ok(_) => {
                self.clip_download_rx = Some(rx);
                self.notice = Some((
                    "Bereite Dateien mit langen relativen Pfaden für die Zwischenablage vor …"
                        .into(),
                    Instant::now(),
                ));
            }
            Err(error) => {
                self.cancel_clipboard_preparation();
                self.error_msg = Some(format!(
                    "Zwischenablage-Vorbereitung konnte nicht starten: {error}"
                ));
            }
        }
    }
}

/// Whether a long-path clipboard of `files` may not be materialized
/// (K05 (3)): one of them lies in a vault drive or is a vault location.
pub(in crate::app) fn materialize_refused(files: &[ClipboardVirtualFile]) -> bool {
    files
        .iter()
        .any(|file| crate::vault::shows_vault(&file.abs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_task_provide_and_materialize_reject_vault_source() {
        let file = |abs: &str| ClipboardVirtualFile {
            abs: abs.to_string(),
            rel: "a".repeat(300),
            size: 1,
            mtime_ms: 0,
        };
        // A published vault drive and a vault location are refused …
        let site = "0123456789abcdef0123456789abcdef";
        let drive = std::env::temp_dir().join(format!("se-vault-drive-{}", std::process::id()));
        crate::vault::publish_vault_mount(site, &drive);
        let inside = drive.join("Geheim").join("Plan.txt");
        assert!(materialize_refused(&[
            file("/home/anna/a.txt"),
            file(&inside.to_string_lossy())
        ]));
        assert!(materialize_refused(&[file(&format!(
            "vault://{site}/Plan.txt"
        ))]));
        // … ordinary local files are not.
        assert!(!materialize_refused(&[file("/home/anna/a.txt")]));
        crate::vault::withdraw_vault_mount(site, &drive);
        assert!(!materialize_refused(&[file(&inside.to_string_lossy())]));
        // „Für andere Programme bereitstellen“ refuses the cleartext view.
        let vault_view = crate::bisync::test_remote::FakeRemote::new(&drive, "tresor")
            .with_scheme(crate::vfs::Scheme::Vault);
        assert!(super::super::clipboard_provide::provide_refused(
            &vault_view
        ));
        let plain = crate::vfs::LocalBackend::new("/");
        assert!(!super::super::clipboard_provide::provide_refused(&plain));
        assert!(crate::vault::PROVIDE_REFUSED.contains("Laufwerk"));
    }
}
