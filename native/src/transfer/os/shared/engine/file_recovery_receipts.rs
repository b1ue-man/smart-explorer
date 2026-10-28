//! Receipt models and the job's durable per-file intents in its crash log
//! (`journal`).
use std::io;

use serde::{Deserialize, Serialize};

use super::{
    destination_parts, failure, FileWork, JobRecord, JobRun, Path, PathBuf, Proof, SourceGuard,
    StoredEndpoint,
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
    /// The target's metadata identity at acknowledgement (no content): a
    /// resumed job keeps an unchanged target without hashing it.
    #[serde(default)]
    pub(super) published: Option<Proof>,
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

pub(super) fn key(file: &FileWork) -> String {
    file.receipt_key.clone().unwrap_or_else(|| {
        format!(
            "{}\0{}",
            file.source,
            file.rel.split_once('/').map_or("", |(_, tail)| tail)
        )
    })
}

/// The job's private folder: its crash log (and, before 0.5.189, one file
/// per receipt).
pub(super) fn directory(job_path: &Path) -> PathBuf {
    job_path.with_extension("files")
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

impl JobRun {
    /// The newest receipt of `file`, from memory or the crash log.
    pub(super) fn read_file(&self, file: &FileWork) -> io::Result<Option<Receipt>> {
        let key = key(file);
        let Some(receipt) = self.journal.get(&key)? else {
            return Ok(None);
        };
        validate(&receipt, &super::super::lock(&self.data))?;
        if receipt.key != key || receipt.source.path != file.source {
            return Err(failure("Dateibeleg passt nicht zur Auswahl"));
        }
        Ok(Some(receipt))
    }

    /// Whether `file` has any receipt in this job (memory only).
    pub(super) fn has_receipt(&self, file: &FileWork) -> bool {
        self.journal.contains(&key(file))
    }

    /// A durable change of a receipt (discarded, revoked, decided).
    pub(super) fn write_file(&self, receipt: &Receipt) -> io::Result<()> {
        self.journal.put(std::slice::from_ref(receipt), false, true)
    }

    /// A publication's acknowledgement: no sync of its own, durable with the
    /// next intent's group commit or the job's end. Lost by a crash, the
    /// file is in doubt and decided when the job resumes.
    pub(super) fn write_published(&self, receipt: &Receipt) -> io::Result<()> {
        self.journal
            .put(std::slice::from_ref(receipt), false, false)
    }

    /// Records the durable intent of one publication.
    pub(super) fn start_file(
        &self,
        file: &FileWork,
        source: SourceGuard,
        staged: Option<Proof>,
        destination: String,
        quarantine: Option<Proof>,
        previous_target: PreviousTarget,
    ) -> io::Result<()> {
        let receipt = self.intent(
            file,
            source,
            staged,
            destination,
            quarantine,
            previous_target,
        )?;
        self.journal.put(std::slice::from_ref(&receipt), true, true)
    }

    /// Records the intents of a whole packet as one durable record.
    pub(super) fn start_files(&self, intents: &[Receipt]) -> io::Result<()> {
        self.journal.put(intents, true, true)
    }

    /// The intent receipt of one publication, not yet written.
    pub(super) fn intent(
        &self,
        file: &FileWork,
        source: SourceGuard,
        staged: Option<Proof>,
        destination: String,
        quarantine: Option<Proof>,
        previous_target: PreviousTarget,
    ) -> io::Result<Receipt> {
        let previous = self.read_file(file)?;
        let data = super::super::lock(&self.data);
        let (relative, destination) = destination_parts(&data, &destination)?;
        // A metadata stage record (empty digest) becomes the publication's
        // evidence only on a local target, whose rename keeps the object.
        let mut proof = staged.clone().filter(|proof| {
            !proof.sha256.is_empty() || (data.target.local && proof.object.is_some())
        });
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
            staged,
            confirmed: false,
            acknowledged: false,
            discarded: false,
            publication_authorized: true,
            previous: Some(previous_target),
            published: None,
        };
        validate(&receipt, &data)?;
        Ok(receipt)
    }
}
