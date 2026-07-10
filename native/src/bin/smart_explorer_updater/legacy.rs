use super::args::LegacyApplyArgs;
#[cfg(test)]
use super::bookkeeping::write_launch_complete;
use super::bookkeeping::{
    launch_complete_matches, launch_complete_path, launch_complete_payload, PreparedBookkeeping,
};
use super::hash::{sha256_file, verify_sha256};
use super::instance;
use super::launch::spawn_verified_acknowledged_receipt;
use super::legacy_intent::LegacyIntent;
use super::legacy_recovery::resolve_interrupted;
use super::logging::append_log;
use super::parent::bind_legacy_parent;
use super::path_safety;
use super::process::stop_target_processes_for_update;
use super::replace::{replace_transaction_with_retries, AppliedTransaction, Replacement};
use super::ApplyFailure;
use std::io::Read;
use std::path::Path;

type ReleaseVersion = (u64, u64, u64);

/// Apply the exact, hash-bound app-only protocol emitted by v0.5.119.
pub(crate) fn apply_update(args: LegacyApplyArgs) -> Result<(), ApplyFailure> {
    let requested_version = parse_release_version(&args.version)?;
    let parent = bind_legacy_parent(args.parent_pid)?;
    append_log(&format!(
        "legacy apply v{}: staged={} target={} parent={}",
        args.version,
        args.staged.display(),
        args.target.display(),
        args.parent_pid
    ));

    let helper = std::env::current_exe()
        .map_err(|error| format!("Updater-Helferpfad unbekannt: {error}"))?;
    validate_paths(&args, &helper)?;
    verify_inputs(&args, &helper)?;
    let mut previous_sha256 = sha256_file(&args.target)?;

    parent.wait()?;
    let _instance = instance::acquire(&args.target)?;
    validate_paths(&args, &helper)?;

    let current_sha256 = sha256_file(&args.target)?;
    if let Some(intent) = LegacyIntent::load(&args.target)? {
        parse_release_version(intent.version())?;
        resolve_interrupted(&args, &intent, &current_sha256)?;
    }
    if let Some((applied_version, applied_number)) = completed_winner(&args, &current_sha256)? {
        if requested_version <= applied_number {
            append_log(&format!(
                "legacy apply v{}: verified serialized winner v{} supersedes this request",
                args.version, applied_version
            ));
            remove_staged(&args.staged);
            return Ok(());
        }
        append_log(&format!(
            "legacy apply v{}: rebasing newer request on verified serialized winner v{} ({})",
            args.version, applied_version, current_sha256
        ));
        previous_sha256 = current_sha256;
    } else {
        if let Some(unproven_version) = read_applied_version(&args.last_applied)? {
            let unproven_number = parse_release_version(&unproven_version)?;
            if unproven_number >= requested_version {
                if unproven_number == requested_version && current_sha256 == args.staged_sha256 {
                    return recover_or_accept_completed_update(&args);
                }
                return Err(format!(
                    "Legacy-Status v{unproven_version} ist nicht durch einen passenden dauerhaften Startabschluss belegt; v{} wird nicht darueber installiert",
                    args.version
                )
                .into());
            }
        }
        if current_sha256 == args.staged_sha256 {
            return recover_or_accept_completed_update(&args);
        }
        if current_sha256 != previous_sha256 {
            return Err(format!(
                "Legacy-Programmziel {} wurde ohne passenden dauerhaften Startabschluss geaendert",
                args.target.display()
            )
            .into());
        }
    }
    verify_inputs(&args, &helper)?;
    verify_sha256(&args.target, &previous_sha256)?;
    let intent = LegacyIntent::create(
        &args.target,
        &args.version,
        &previous_sha256,
        &args.staged_sha256,
    )?;
    if let Err(error) = stop_target_processes_for_update(&args.target) {
        let message = if error.needs_elevation {
            elevation_refused(&args, "Prozessbereinigung")
        } else {
            error.msg
        };
        return Err(abort_unstarted_update(
            &intent,
            &args,
            &previous_sha256,
            message,
        ));
    }
    if let Err(error) = verify_sha256(&args.target, &previous_sha256) {
        return Err(abort_unstarted_update(
            &intent,
            &args,
            &previous_sha256,
            format!("Programmziel vor dem Ersetzen veraendert: {error}"),
        ));
    }

    let replacements = [Replacement {
        label: "Smart Explorer (Legacy-Protokoll)",
        staged: &args.staged,
        target: &args.target,
        sha256: &args.staged_sha256,
        expected_target_sha256: Some(&previous_sha256),
    }];
    let mut transaction = match replace_transaction_with_retries(&replacements) {
        Ok(transaction) => transaction,
        Err(error) if error.needs_elevation => {
            return Err(abort_unstarted_update(
                &intent,
                &args,
                &previous_sha256,
                elevation_refused(&args, "Programmdatei ersetzen"),
            ));
        }
        Err(error) => {
            return Err(abort_unstarted_update(
                &intent,
                &args,
                &previous_sha256,
                format!("Legacy-Update-Transaktion: {}", error.msg),
            ));
        }
    };

    if let Err(error) = verify_sha256(&args.target, &args.staged_sha256) {
        return Err(restore_failed_update(
            &args,
            transaction,
            &intent,
            &previous_sha256,
            format!("Neue Programmdatei nach dem Ersetzen ungueltig: {error}"),
        ));
    }
    let target_key = instance::target_key(&args.target);
    let marker = launch_complete_path(&args.last_applied, &target_key)?;
    let mut bookkeeping = match PreparedBookkeeping::prepare(
        &args.last_applied,
        &args.error_file,
        &args.version,
        &[&marker],
    ) {
        Ok(bookkeeping) => bookkeeping,
        Err(error) => {
            return Err(restore_failed_update(
                &args,
                transaction,
                &intent,
                &previous_sha256,
                format!("Legacy-Update-Status vor dem Neustart vorbereiten: {error}"),
            ));
        }
    };
    let receipt = launch_complete_payload(&target_key, &args.version, &args.staged_sha256)?;
    if let Err(launch_error) = spawn_verified_acknowledged_receipt(
        &args.target,
        &args.staged_sha256,
        &["--updated"],
        &marker,
        &receipt,
    ) {
        let status_error = bookkeeping.rollback().err();
        return Err(restore_failed_update(
            &args,
            transaction,
            &intent,
            &previous_sha256,
            format!(
                "Neue Version konnte nicht gestartet werden: {launch_error}{}",
                status_error
                    .as_deref()
                    .map(|error| format!("; Status-Rollback fehlgeschlagen: {error}"))
                    .unwrap_or_default()
            ),
        ));
    }

    if let Err(error) = require_completion(&marker, &target_key, &args) {
        std::mem::forget(bookkeeping);
        std::mem::forget(transaction);
        return Err(format!(
            "Neue Version wurde bestaetigt, aber der dauerhafte Startabschluss ist ungueltig; Wiederherstellungszustand bleibt erhalten: {error}"
        )
        .into());
    }
    transaction.finalize();
    finish_bookkeeping(&args, &mut bookkeeping);
    if let Err(error) = intent.clear() {
        append_log(&format!(
            "warning: abgeschlossene Legacy-Update-Absicht bleibt erhalten: {error}"
        ));
    }
    append_log(&format!("legacy apply v{}: ok", args.version));
    Ok(())
}

fn completed_winner(
    args: &LegacyApplyArgs,
    current_sha256: &str,
) -> Result<Option<(String, ReleaseVersion)>, String> {
    let Some(applied_version) = read_applied_version(&args.last_applied)? else {
        return Ok(None);
    };
    let target_key = instance::target_key(&args.target);
    let marker = launch_complete_path(&args.last_applied, &target_key)?;
    if !launch_complete_matches(&marker, &target_key, &applied_version, current_sha256)? {
        return Ok(None);
    }
    verify_sha256(&args.target, current_sha256)?;
    let applied_number = parse_release_version(&applied_version)?;
    Ok(Some((applied_version, applied_number)))
}

fn read_applied_version(path: &Path) -> Result<Option<String>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && metadata.len() <= 64 => metadata,
        Ok(metadata) if metadata.file_type().is_file() => {
            return Err(format!(
                "Legacy-Update-Status {} ist zu gross",
                path.display()
            ));
        }
        Ok(_) => {
            return Err(format!(
                "Legacy-Update-Status {} ist keine Datei",
                path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Legacy-Update-Status {} lesen: {error}",
                path.display()
            ))
        }
    };
    let mut raw = String::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)
        .and_then(|file| file.take(65).read_to_string(&mut raw))
        .map_err(|error| format!("Legacy-Update-Status {} lesen: {error}", path.display()))?;
    let version = raw.trim();
    Ok((!version.is_empty()).then(|| version.to_string()))
}

fn parse_release_version(value: &str) -> Result<ReleaseVersion, String> {
    if value.trim() != value {
        return Err(format!("Release-Version {value:?} ist ungueltig"));
    }
    let value = value.strip_prefix('v').unwrap_or(value);
    let mut parts = value.split('.');
    let mut next = || -> Result<u64, String> {
        let part = parts
            .next()
            .ok_or_else(|| format!("Release-Version {value:?} ist unvollstaendig"))?;
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return Err(format!("Release-Version {value:?} ist ungueltig"));
        }
        part.parse::<u64>()
            .map_err(|error| format!("Release-Version {value:?} ist ungueltig: {error}"))
    };
    let version = (next()?, next()?, next()?);
    if parts.next().is_some() {
        return Err(format!("Release-Version {value:?} ist ungueltig"));
    }
    Ok(version)
}

fn verify_inputs(args: &LegacyApplyArgs, helper: &Path) -> Result<(), String> {
    if let Some(expected) = args.helper_sha256.as_deref() {
        verify_sha256(helper, expected)?;
    } else {
        append_log(
            "legacy caller supplied no helper SHA; compatibility mode remains non-elevating",
        );
    }
    verify_sha256(&args.staged, &args.staged_sha256)
}

fn elevation_refused(args: &LegacyApplyArgs, operation: &str) -> String {
    format!(
        "{operation} benoetigt Administratorrechte; das Legacy-Protokoll besitzt keinen sicheren UAC-Handoff, bitte den v{}-Installer verwenden",
        args.version
    )
}

fn restore_failed_update(
    args: &LegacyApplyArgs,
    mut transaction: AppliedTransaction,
    intent: &LegacyIntent,
    previous_sha256: &str,
    failure: String,
) -> ApplyFailure {
    let rollback_error = transaction.rollback().err();
    if let Err(error) = verify_sha256(&args.target, previous_sha256) {
        return format!(
            "{failure}; Wiederherstellung der vorherigen Version fehlgeschlagen{}: {error}",
            rollback_error
                .as_deref()
                .map(|rollback| format!(" ({rollback})"))
                .unwrap_or_default()
        )
        .into();
    }
    let intent_error = intent.clear().err();
    let reported = format!(
        "{failure}; vorherige Version wurde wiederhergestellt und wird neu gestartet{}{}",
        rollback_error
            .as_deref()
            .map(|rollback| format!(" (Transaktions-Rollback meldete: {rollback})"))
            .unwrap_or_default(),
        intent_error
            .as_deref()
            .map(|error| format!("; Update-Absicht konnte nicht entfernt werden: {error}"))
            .unwrap_or_default()
    );
    ApplyFailure::with_recovery(reported, &args.target, previous_sha256)
}

fn abort_unstarted_update(
    intent: &LegacyIntent,
    args: &LegacyApplyArgs,
    previous_sha256: &str,
    failure: String,
) -> ApplyFailure {
    if let Err(error) = verify_sha256(&args.target, previous_sha256) {
        return format!(
            "{failure}; Programmziel ist nicht mehr die gebundene Ausgangsversion ({error}), dauerhafte Update-Absicht bleibt erhalten"
        )
        .into();
    }
    match intent.clear() {
        Ok(()) => failure.into(),
        Err(error) => format!(
            "{failure}; unveraendertes Programmziel bestaetigt, aber Update-Absicht konnte nicht entfernt werden: {error}"
        )
        .into(),
    }
}

fn recover_or_accept_completed_update(args: &LegacyApplyArgs) -> Result<(), ApplyFailure> {
    let target_key = instance::target_key(&args.target);
    let marker = launch_complete_path(&args.last_applied, &target_key)?;
    if std::fs::read_to_string(&args.last_applied)
        .is_ok_and(|version| version.trim() == args.version)
        && launch_complete_matches(&marker, &target_key, &args.version, &args.staged_sha256)?
    {
        verify_sha256(&args.target, &args.staged_sha256)?;
        append_log(&format!(
            "legacy apply v{}: another serialized worker already completed",
            args.version
        ));
        remove_staged(&args.staged);
        return Ok(());
    }

    append_log(&format!(
        "legacy apply v{}: recovering a verified replacement left before launch",
        args.version
    ));
    let mut bookkeeping = PreparedBookkeeping::prepare(
        &args.last_applied,
        &args.error_file,
        &args.version,
        &[&marker],
    )?;
    let receipt = launch_complete_payload(&target_key, &args.version, &args.staged_sha256)?;
    if let Err(error) = spawn_verified_acknowledged_receipt(
        &args.target,
        &args.staged_sha256,
        &["--updated"],
        &marker,
        &receipt,
    ) {
        let rollback = bookkeeping.rollback().err();
        return Err(format!(
            "Bereits ersetzte Legacy-Version konnte nicht gestartet werden: {error}{}",
            rollback
                .as_deref()
                .map(|rollback| format!("; Status-Rollback fehlgeschlagen: {rollback}"))
                .unwrap_or_default()
        )
        .into());
    }
    if let Err(error) = require_completion(&marker, &target_key, args) {
        std::mem::forget(bookkeeping);
        return Err(format!(
            "Bereits ersetzte Legacy-Version wurde bestaetigt, aber der dauerhafte Startabschluss ist ungueltig; Statussicherungen bleiben erhalten: {error}"
        )
        .into());
    }
    finish_bookkeeping(args, &mut bookkeeping);
    Ok(())
}

fn require_completion(
    marker: &Path,
    target_key: &str,
    args: &LegacyApplyArgs,
) -> Result<(), String> {
    match launch_complete_matches(marker, target_key, &args.version, &args.staged_sha256) {
        Ok(true) => Ok(()),
        Ok(false) => {
            Err("Startabschluss stimmt nicht mit Ziel, Version und SHA-256 ueberein".into())
        }
        Err(error) => Err(error),
    }
}

fn finish_bookkeeping(args: &LegacyApplyArgs, bookkeeping: &mut PreparedBookkeeping) {
    for warning in bookkeeping.commit() {
        append_log(&format!("warning: {warning}"));
    }
    remove_staged(&args.staged);
}

fn remove_staged(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            append_log(&format!(
                "warning: gestagte App {} entfernen: {error}",
                path.display()
            ));
        }
    }
}

fn validate_paths(args: &LegacyApplyArgs, helper: &Path) -> Result<(), String> {
    let target_key = instance::target_key(&args.target);
    let marker = launch_complete_path(&args.last_applied, &target_key)?;
    let intent = LegacyIntent::path_for(&args.target);
    path_safety::validate_distinct_paths(&[
        ("Programmziel", &args.target),
        ("gestagte App", &args.staged),
        ("Updater-Helfer", helper),
        ("Update-Status", &args.last_applied),
        ("Startabschluss", &marker),
        ("Update-Absicht", &intent),
        ("Fehlerstatus", &args.error_file),
    ])
}

#[cfg(test)]
#[path = "legacy_tests.rs"]
mod tests;
