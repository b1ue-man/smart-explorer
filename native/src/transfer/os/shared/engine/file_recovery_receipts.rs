//! Receipt models, complete journal indexes and durable per-file intents.
use std::io;

use ring::digest::SHA256;
use serde::{Deserialize, Serialize};

use super::{
    destination_parts, failure, read_record, write_record, FileWork, JobRecord, JobRun, Path,
    PathBuf, Proof, RecordContext, RecordKind, SourceGuard, StoredEndpoint,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Receipt {
    pub(super) version: u32,
    pub(super) job: String,
    pub(super) source_endpoint: StoredEndpoint,
    pub(super) target_endpoint: StoredEndpoint,
    pub(super) target_root: String,
    pub(super) key: String,
    pub(super) source: SourceGuard,
    pub(super) relative: String,
    pub(super) destination: String,
    pub(super) proof: Option<Proof>,
    #[serde(default)]
    pub(super) quarantine: Option<Proof>,
    #[serde(default)]
    pub(super) staged: Option<Proof>,
    pub(super) confirmed: bool,
    /// A successful current execution is separate from persistent ownership.
    #[serde(default)]
    pub(super) acknowledged: bool,
    #[serde(default)]
    pub(super) discarded: bool,
    #[serde(default = "authorized_default")]
    pub(super) publication_authorized: bool,
    #[serde(default)]
    pub(super) previous: Option<PreviousTarget>,
}

pub(super) fn authorized_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum PreviousTarget {
    Absent,
    File(Proof),
    Unconfirmed,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Index {
    pub(super) job: String,
    pub(super) source: StoredEndpoint,
    pub(super) target: StoredEndpoint,
    pub(super) root: String,
    pub(super) count: u64,
    #[serde(default)]
    pub(super) pending: Option<Receipt>,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn key(file: &FileWork) -> String {
    file.receipt_key.clone().unwrap_or_else(|| {
        format!(
            "{}\0{}",
            file.source,
            file.rel.split_once('/').map_or("", |(_, tail)| tail)
        )
    })
}

pub(super) fn directory(job_path: &Path) -> PathBuf {
    job_path.with_extension("files")
}

pub(in crate::transfer::engine) fn initialize(
    job_path: &Path,
    record: &JobRecord,
    context: Option<&RecordContext>,
) -> io::Result<()> {
    crate::support_dirs::ensure_private_dir(&directory(job_path))?;
    match read_index(job_path, record, context) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound && !record.mutation_started => {
            write_record(
                &directory(job_path).join("index.json"),
                RecordKind::Job,
                &record.id,
                context,
                &Index {
                    job: record.id.clone(),
                    source: record.source.clone(),
                    target: record.target.clone(),
                    root: record.target_dir.clone(),
                    count: 0,
                    pending: None,
                },
            )
        }
        Err(error) => Err(error),
    }
}

pub(super) fn read_index(
    job_path: &Path,
    record: &JobRecord,
    context: Option<&RecordContext>,
) -> io::Result<Index> {
    let index: Index = read_record(
        &directory(job_path).join("index.json"),
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

pub(super) fn receipt_path(job_path: &Path, key: &str) -> PathBuf {
    directory(job_path).join(format!("{}.json", digest(key.as_bytes())))
}

pub(super) fn repair_new_entry(
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
        if entry.file_name() == "index.json"
            || entry.path().extension().and_then(|ext| ext.to_str()) != Some("json")
        {
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
        &directory(job_path).join("index.json"),
        RecordKind::Job,
        &record.id,
        context,
        index,
    )
}

pub(super) fn validate(receipt: &Receipt, record: &JobRecord) -> io::Result<()> {
    if receipt.version != 1
        || receipt.job != record.id
        || receipt.source_endpoint != record.source
        || receipt.target_endpoint != record.target
        || receipt.target_root != record.target_dir
    {
        return Err(failure(
            "Dateibeleg gehört zu einer abweichenden Verbindung oder einem abweichenden Auftrag",
        ));
    }
    super::paths::validate(receipt, record)
}

pub(super) fn visit_saved(
    record: &JobRecord,
    job_path: &Path,
    context: Option<&RecordContext>,
    mut visit: impl FnMut(Receipt) -> io::Result<()>,
) -> io::Result<()> {
    let mut index = read_index(job_path, record, context)?;
    repair_new_entry(job_path, record, context, &mut index)?;
    let mut count = 0u64;
    let entries = match std::fs::read_dir(directory(job_path)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !record.mutation_started => {
            return Ok(())
        }
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name() == "index.json"
            || entry.path().extension().and_then(|ext| ext.to_str()) != Some("json")
        {
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

impl JobRun {
    pub(super) fn read_file(&self, file: &FileWork) -> io::Result<Option<Receipt>> {
        let data = super::super::lock(&self.data);
        self.read_file_unlocked(file, &data)
    }
    pub(super) fn read_file_unlocked(
        &self,
        file: &FileWork,
        data: &JobRecord,
    ) -> io::Result<Option<Receipt>> {
        let path = receipt_path(&self.path, &key(file));
        match read_record::<Receipt>(
            &path,
            RecordKind::Job,
            &data.id,
            u64::MAX,
            self.context.is_some(),
        ) {
            Ok(receipt) => {
                validate(&receipt, &data)?;
                if receipt.key != key(file) || receipt.source.path != file.source {
                    return Err(failure("Dateibeleg passt nicht zur Auswahl"));
                }
                Ok(Some(receipt))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub(super) fn write_file(&self, receipt: &Receipt) -> io::Result<()> {
        write_record(
            &receipt_path(&self.path, &receipt.key),
            RecordKind::Job,
            &receipt.job,
            self.context.as_ref(),
            receipt,
        )
    }
    pub(super) fn start_file(
        &self,
        file: &FileWork,
        source: SourceGuard,
        staged: Option<Proof>,
        destination: String,
        quarantine: Option<Proof>,
        previous_target: PreviousTarget,
    ) -> io::Result<()> {
        let data = super::super::lock(&self.data);
        let (relative, destination) = destination_parts(&data, &destination)?;
        let mut index = read_index(&self.path, &data, self.context.as_ref())?;
        repair_new_entry(&self.path, &data, self.context.as_ref(), &mut index)?;
        let previous = self.read_file_unlocked(file, &data)?;
        let new_key = previous.is_none();
        let stage = staged.clone();
        let mut proof = staged;
        if let Some(proof) = &mut proof {
            proof.path = destination.clone();
        }
        let mut previous_target = previous_target;
        if let PreviousTarget::File(proof) = &mut previous_target {
            proof.path = destination.clone();
        }
        let receipt = Receipt {
            version: 1,
            job: data.id.clone(),
            source_endpoint: data.source.clone(),
            target_endpoint: data.target.clone(),
            target_root: data.target_dir.clone(),
            key: key(file),
            source,
            relative,
            destination,
            proof,
            quarantine: quarantine.or_else(|| previous.and_then(|receipt| receipt.quarantine)),
            staged: stage,
            confirmed: false,
            acknowledged: false,
            discarded: false,
            publication_authorized: true,
            previous: Some(previous_target),
        };
        validate(&receipt, &data)?;
        if new_key {
            index.pending = Some(receipt.clone());
            write_record(
                &directory(&self.path).join("index.json"),
                RecordKind::Job,
                &data.id,
                self.context.as_ref(),
                &index,
            )?;
        }
        self.write_file(&receipt)?;
        if new_key {
            index.pending = None;
            index.count = index
                .count
                .checked_add(1)
                .ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?;
            write_record(
                &directory(&self.path).join("index.json"),
                RecordKind::Job,
                &data.id,
                self.context.as_ref(),
                &index,
            )?;
        }
        Ok(())
    }
}
