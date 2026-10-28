//! The journal of 0.5.188 and earlier: `index.json` (job binding, receipt
//! count and the one pending new-key intent) next to one sealed or plain
//! JSON file per receipt. It is read once, converted into the crash log
//! (`journal`) and removed. Its pending intent is repaired exactly as
//! before (it becomes a never-authorized receipt), and a receipt lost since
//! its count was committed still fails the job.
use std::ffi::OsStr;
use std::io;

use ring::digest::SHA256;
use serde::{Deserialize, Serialize};

use super::receipts::{directory, validate, Receipt};
use super::{
    failure, read_record, write_record, JobRecord, Path, PathBuf, RecordContext, RecordKind,
    StoredEndpoint,
};

const INDEX: &str = "index.json";

#[derive(Serialize, Deserialize)]
struct Index {
    job: String,
    source: StoredEndpoint,
    target: StoredEndpoint,
    root: String,
    count: u64,
    #[serde(default)]
    pending: Option<Receipt>,
}

fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn receipt_path(job_path: &Path, key: &str) -> PathBuf {
    directory(job_path).join(format!("{}.json", digest(key.as_bytes())))
}

fn is_receipt_file(path: &Path) -> bool {
    path.file_name() != Some(OsStr::new(INDEX))
        && path.extension().and_then(|ext| ext.to_str()) == Some("json")
}

/// Whether the job has a journal of the older format.
pub(super) fn present(job_path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(directory(job_path).join(INDEX)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Receipt files without their index: a damaged older journal, never a
/// fresh start.
pub(super) fn stray(job_path: &Path) -> io::Result<bool> {
    let entries = match std::fs::read_dir(directory(job_path)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for entry in entries {
        if is_receipt_file(&entry?.path()) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Removes a converted older journal: the receipts first, the index last,
/// so a crash in between still shows the conversion as unfinished cleanup
/// (the crash log, once promoted, always wins).
pub(super) fn remove(job_path: &Path) {
    let folder = directory(job_path);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return;
    };
    let mut index = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if is_receipt_file(&path) {
            let _ = std::fs::remove_file(path);
        } else if path.file_name() == Some(OsStr::new(INDEX)) {
            index = true;
        }
    }
    if index {
        let _ = std::fs::remove_file(folder.join(INDEX));
    }
}

fn read_index(
    job_path: &Path,
    record: &JobRecord,
    context: Option<&RecordContext>,
) -> io::Result<Index> {
    let index: Index = read_record(
        &directory(job_path).join(INDEX),
        RecordKind::Job,
        &record.id,
        u64::MAX,
        context.is_some(),
    )?;
    if index.job != record.id
        || index.source != record.source
        || index.target != record.target
        || index.root != record.target_dir
    {
        return Err(failure(
            "Dateijournalindex gehört zu einem abweichenden Auftrag",
        ));
    }
    Ok(index)
}

fn repair_new_entry(
    job_path: &Path,
    record: &JobRecord,
    context: Option<&RecordContext>,
    index: &mut Index,
) -> io::Result<()> {
    let Some(intent) = index.pending.clone() else {
        return Ok(());
    };
    validate(&intent, record)?;
    let candidate = receipt_path(job_path, &intent.key);
    let mut unapproved = intent.clone();
    unapproved.publication_authorized = false;
    let mut committed = 0u64;
    for entry in std::fs::read_dir(directory(job_path))? {
        let entry = entry?;
        if !is_receipt_file(&entry.path()) {
            continue;
        }
        let receipt: Receipt = read_record(
            &entry.path(),
            RecordKind::Job,
            &record.id,
            u64::MAX,
            context.is_some(),
        )?;
        validate(&receipt, record)?;
        if receipt_path(job_path, &receipt.key) != entry.path() {
            return Err(failure("Abweichender Dateijournalschlüssel"));
        }
        if entry.path() == candidate {
            if receipt != intent && receipt != unapproved {
                return Err(failure("Unbestätigter Journalneueintrag wurde verändert"));
            }
        } else {
            committed = committed
                .checked_add(1)
                .ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?;
        }
    }
    // The intent identifies exactly one never-authorized new key. Every
    // previously committed key must still be present; never recount loss.
    if committed != index.count {
        return Err(failure(
            "Bestehende Dateibelege fehlen während Journalwiederaufnahme",
        ));
    }
    write_record(
        &candidate,
        RecordKind::Job,
        &record.id,
        context,
        &unapproved,
    )?;
    index.count = index
        .count
        .checked_add(1)
        .ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?;
    index.pending = None;
    write_record(
        &directory(job_path).join(INDEX),
        RecordKind::Job,
        &record.id,
        context,
        index,
    )
}

/// Every receipt of the older journal, after the pending repair; fails when
/// a counted receipt is missing or an uncounted one appeared.
pub(super) fn visit(
    job_path: &Path,
    record: &JobRecord,
    context: Option<&RecordContext>,
    mut visit: impl FnMut(Receipt) -> io::Result<()>,
) -> io::Result<()> {
    let mut index = read_index(job_path, record, context)?;
    repair_new_entry(job_path, record, context, &mut index)?;
    let mut count = 0u64;
    for entry in std::fs::read_dir(directory(job_path))? {
        let entry = entry?;
        if !is_receipt_file(&entry.path()) {
            continue;
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?;
        let receipt: Receipt = read_record(
            &entry.path(),
            RecordKind::Job,
            &record.id,
            u64::MAX,
            context.is_some(),
        )?;
        validate(&receipt, record)?;
        if receipt_path(job_path, &receipt.key) != entry.path() {
            return Err(failure(
                "Dateibeleg hat einen abweichenden Speicherschlüssel",
            ));
        }
        visit(receipt)?;
    }
    if count != index.count {
        return Err(failure(
            "Dateijournal ist unvollständig oder enthält unbestätigte Belege",
        ));
    }
    Ok(())
}

/// Rewrites the closed crash log of the job stored at `job_path` as the
/// journal of 0.5.188 and earlier (tests of the conversion): every receipt
/// its own counted file, except the one of `pending` (a file, and whether
/// its receipt file stays), which becomes the index's pending intent.
#[cfg(test)]
pub(in crate::transfer::engine) fn convert_to_old_format(
    job_path: &Path,
    record: &JobRecord,
    pending: Option<(&super::FileWork, bool)>,
) -> io::Result<()> {
    let mut saved = Vec::new();
    super::Journal::open(job_path, record, None, false)?.visit(|receipt| {
        saved.push(receipt);
        Ok(())
    })?;
    std::fs::remove_file(super::log_path(job_path))?;
    let pending = pending.map(|(file, keep)| (super::receipts::key(file), keep));
    crate::support_dirs::ensure_private_dir(&directory(job_path))?;
    let mut index = Index {
        job: record.id.clone(),
        source: record.source.clone(),
        target: record.target.clone(),
        root: record.target_dir.clone(),
        count: 0,
        pending: None,
    };
    for receipt in &saved {
        match &pending {
            Some((key, keep)) if *key == receipt.key => {
                index.pending = Some(receipt.clone());
                if !*keep {
                    continue;
                }
            }
            _ => index.count += 1,
        }
        write_record(
            &receipt_path(job_path, &receipt.key),
            RecordKind::Job,
            &record.id,
            None,
            receipt,
        )?;
    }
    write_record(
        &directory(job_path).join(INDEX),
        RecordKind::Job,
        &record.id,
        None,
        &index,
    )
}

/// The older format's receipt file of `file` (tests of the conversion).
#[cfg(test)]
pub(in crate::transfer::engine) fn old_receipt_file(
    job_path: &Path,
    file: &super::FileWork,
) -> PathBuf {
    receipt_path(job_path, &super::receipts::key(file))
}
