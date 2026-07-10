use super::args::LegacyApplyArgs;
use super::bookkeeping::{
    launch_complete_matches, launch_complete_path, launch_complete_payload, PreparedBookkeeping,
};
use super::hash::verify_sha256;
use super::launch::spawn_verified_acknowledged_receipt;
use super::legacy_intent::LegacyIntent;
use super::logging::append_log;
use super::process::target_is_running;
use super::ApplyFailure;
use std::io::Read;
use std::time::{Duration, Instant};

const EXISTING_LAUNCH_WAIT: Duration = Duration::from_secs(45);

pub(crate) fn resolve_interrupted(
    args: &LegacyApplyArgs,
    intent: &LegacyIntent,
    current_sha256: &str,
) -> Result<(), ApplyFailure> {
    let marker = launch_complete_path(&args.last_applied, intent.target_key())?;
    if current_sha256 != intent.staged_sha256() {
        let state = if current_sha256 == intent.previous_sha256() {
            "die bisherige Programmdatei ist noch installiert"
        } else {
            "das Programmziel hat eine dritte Pruefsumme"
        };
        return Err(format!(
            "Unabgeschlossene Legacy-Aktualisierung v{} bleibt gesperrt ({state}); bitte den v{}-Installer verwenden",
            intent.version(),
            intent.version()
        )
        .into());
    }

    append_log(&format!(
        "legacy apply: recovering durable interrupted intent for v{}",
        intent.version()
    ));
    verify_sha256(&args.target, intent.staged_sha256())?;
    if wait_for_existing_launch(args, intent, &marker, EXISTING_LAUNCH_WAIT)? {
        intent.clear()?;
        append_log(&format!(
            "legacy apply: cleared completed intent for v{}",
            intent.version()
        ));
        return Ok(());
    }
    let mut bookkeeping = PreparedBookkeeping::prepare(
        &args.last_applied,
        &args.error_file,
        intent.version(),
        &[&marker],
    )?;
    let receipt = launch_complete_payload(
        intent.target_key(),
        intent.version(),
        intent.staged_sha256(),
    )?;
    if let Err(error) = spawn_verified_acknowledged_receipt(
        &args.target,
        intent.staged_sha256(),
        &["--updated"],
        &marker,
        &receipt,
    ) {
        let rollback = bookkeeping.rollback().err();
        return Err(format!(
            "Unterbrochene Legacy-Aktualisierung v{} konnte nicht bestaetigt werden: {error}{}; dauerhafte Absicht bleibt erhalten",
            intent.version(),
            rollback
                .as_deref()
                .map(|rollback| format!("; Status-Rollback fehlgeschlagen: {rollback}"))
                .unwrap_or_default()
        )
        .into());
    }
    let completion = launch_complete_matches(
        &marker,
        intent.target_key(),
        intent.version(),
        intent.staged_sha256(),
    );
    match completion {
        Ok(true) => {}
        Ok(false) => {
            std::mem::forget(bookkeeping);
            return Err(format!(
                "Unterbrochene Legacy-Aktualisierung v{} meldete keinen gueltigen dauerhaften Startabschluss; Absicht und Statussicherungen bleiben erhalten",
                intent.version()
            )
            .into());
        }
        Err(error) => {
            std::mem::forget(bookkeeping);
            return Err(format!(
                "Dauerhafter Startabschluss der unterbrochenen Legacy-Aktualisierung v{} konnte nicht geprueft werden: {error}; Absicht und Statussicherungen bleiben erhalten",
                intent.version()
            )
            .into());
        }
    }
    for warning in bookkeeping.commit() {
        append_log(&format!("warning: {warning}"));
    }
    intent.clear()?;
    Ok(())
}

fn wait_for_existing_launch(
    args: &LegacyApplyArgs,
    intent: &LegacyIntent,
    marker: &std::path::Path,
    timeout: Duration,
) -> Result<bool, ApplyFailure> {
    let deadline = Instant::now() + timeout;
    loop {
        if completion_proved(args, intent, marker)? {
            return Ok(true);
        }
        let running = target_is_running(&args.target).map_err(|error| {
            format!(
                "Laufende Zielversion der unterbrochenen Legacy-Aktualisierung konnte nicht sicher geprueft werden: {}",
                error.msg
            )
        })?;
        if !running {
            return Ok(false);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Smart Explorer v{} laeuft ohne pruefbaren Startabschluss; keine zweite Instanz wird gestartet, bitte den Installer verwenden",
                intent.version()
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn completion_proved(
    args: &LegacyApplyArgs,
    intent: &LegacyIntent,
    marker: &std::path::Path,
) -> Result<bool, String> {
    if !launch_complete_matches(
        marker,
        intent.target_key(),
        intent.version(),
        intent.staged_sha256(),
    )? {
        return Ok(false);
    }
    if !status_matches(args, intent.version())? {
        return Err(format!(
            "Startabschluss fuer v{} ist vorhanden, aber der dauerhafte Versionsstatus stimmt nicht ueberein",
            intent.version()
        ));
    }
    verify_sha256(&args.target, intent.staged_sha256())?;
    Ok(true)
}

fn status_matches(args: &LegacyApplyArgs, expected: &str) -> Result<bool, String> {
    let metadata = match std::fs::symlink_metadata(&args.last_applied) {
        Ok(metadata) if metadata.file_type().is_file() && metadata.len() <= 64 => metadata,
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Legacy-Update-Status {} pruefen: {error}",
                args.last_applied.display()
            ));
        }
    };
    let mut raw = String::with_capacity(metadata.len() as usize);
    std::fs::File::open(&args.last_applied)
        .and_then(|file| file.take(65).read_to_string(&mut raw))
        .map_err(|error| {
            format!(
                "Legacy-Update-Status {} lesen: {error}",
                args.last_applied.display()
            )
        })?;
    Ok(raw.trim() == expected)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::bookkeeping::{launch_complete_path, write_launch_complete};
    use crate::hash::sha256_file;

    fn args_for(target: &std::path::Path, dir: &std::path::Path) -> LegacyApplyArgs {
        let staged = dir.join("staged");
        std::fs::write(&staged, b"staged").unwrap();
        LegacyApplyArgs {
            target: target.to_path_buf(),
            staged: staged.clone(),
            staged_sha256: sha256_file(&staged).unwrap(),
            helper_sha256: None,
            parent_pid: 0,
            version: "0.5.121".into(),
            last_applied: dir.join("last.txt"),
            error_file: dir.join("error.txt"),
        }
    }

    #[test]
    fn existing_receipt_is_accepted_without_relaunch() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        std::fs::write(&target, b"winner").unwrap();
        let hash = sha256_file(&target).unwrap();
        let intent = LegacyIntent::create(&target, "0.5.122", &"a".repeat(64), &hash).unwrap();
        let args = args_for(&target, dir.path());
        std::fs::write(&args.last_applied, b"0.5.122").unwrap();
        let marker = launch_complete_path(&args.last_applied, intent.target_key()).unwrap();
        write_launch_complete(&marker, intent.target_key(), intent.version(), &hash).unwrap();

        assert!(wait_for_existing_launch(&args, &intent, &marker, Duration::ZERO).unwrap());
        intent.clear().unwrap();
    }

    #[test]
    fn running_target_without_receipt_is_never_launched_twice() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        std::fs::copy("/bin/sleep", &target).unwrap();
        let hash = sha256_file(&target).unwrap();
        let intent = LegacyIntent::create(&target, "0.5.122", &"a".repeat(64), &hash).unwrap();
        let args = args_for(&target, dir.path());
        let marker = launch_complete_path(&args.last_applied, intent.target_key()).unwrap();
        let mut child = std::process::Command::new(&target)
            .arg("2")
            .spawn()
            .unwrap();

        let result = wait_for_existing_launch(&args, &intent, &marker, Duration::from_millis(100));

        assert!(result.is_err());
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
        intent.clear().unwrap();
    }
}
