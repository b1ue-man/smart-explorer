//! Transfer evidence of the SMB backend (G3-02 for SMB, 2026-10-28): what
//! the server already answered while a copy stage was written and published
//! replaces the receipts' own stats (`vfs::take_stage_evidence`,
//! `vfs::promote_copy_stage_evidence`), as SFTP does (sftp stage_evidence.rs,
//! whose store `WrittenStages` this backend shares).
//!
//! - An engine copy stage's writer (unsynced commit) sends the stat of the
//!   destination together with its CLOSE and the stat of the stage right
//!   after it: one round trip instead of the receipts' two.
//! - The publication is one compound: CREATE of the stage (no folder, a link
//!   opened as itself), SET_INFO(rename, ReplaceIfExists = 0) and CLOSE. The
//!   CREATE answer describes the very file that now carries the destination
//!   name (a rename keeps size and write time); only a regular file of the
//!   stage's size counts as evidence. A taken name fails the SET_INFO with
//!   OBJECT_NAME_COLLISION (`AlreadyExists`), so the engine numbers it.
//!
//! Evidence is what `stat` (`wire::stat`, `listing::meta`) answers: SMB has
//! no object identity here (`vfs::file_identity` is `None`). Anything else
//! (no answer, a reparse point, another size) is no evidence, and the engine
//! asks as before.
use super::listing::{self, Attributes};
use super::replace::{rename_info, FILE_RENAME_INFORMATION};
use super::wire;
use crate::sftp::WrittenStages;
use crate::vfs::{FileEvidence, StageEvidence};
use smb2::client::Connection;
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, FileId};
use smb2::{CompoundOp, ErrorKind, Tree};

/// Evidence of `path` from attributes the server answered: only a regular
/// file (no folder, no reparse point of any kind).
pub(super) fn regular_evidence(path: &str, attributes: &Attributes) -> Option<FileEvidence> {
    if attributes.is_dir() || attributes.is_reparse_point() {
        return None;
    }
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    let meta = listing::meta(name, attributes);
    Some(FileEvidence {
        path: path.to_string(),
        object: None,
        id: meta.id,
        size: meta.size,
        mtime_ms: meta.mtime_ms,
    })
}

/// Whether a stat proves something is at the path (`Some(true)`) or nothing
/// is (`Some(false)`); any other failure proves neither.
pub(super) fn presence(answer: &smb2::Result<Attributes>) -> Option<bool> {
    match answer {
        Ok(_) => Some(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// What a copy stage's writer reports at its commit.
pub(super) struct StageReport {
    pub(super) stage: String,
    pub(super) destination: String,
    pub(super) destination_rel: String,
    pub(super) stages: WrittenStages,
}

impl StageReport {
    /// A report for an engine upload stage (`<destination>.se-upload-<16
    /// hex>`); `None` for any other name.
    pub(super) fn for_stage(stage: &str, stages: &WrittenStages) -> Option<Self> {
        let destination = crate::sftp::upload_stage_destination(stage)?;
        let destination_rel = super::url::entry_path(&destination).ok()?.rel;
        Some(Self {
            stage: stage.to_string(),
            destination,
            destination_rel,
            stages: stages.clone(),
        })
    }

    /// Keeps the stage's evidence: `stage` is its stat after the CLOSE,
    /// `destination` the stat of the destination sent with the CLOSE,
    /// `written` the bytes the server confirmed.
    pub(super) fn record(
        &self,
        stage: &smb2::Result<Attributes>,
        destination: Option<&smb2::Result<Attributes>>,
        written: u64,
    ) {
        let Some(evidence) = stage
            .as_ref()
            .ok()
            .and_then(|attributes| regular_evidence(&self.stage, attributes))
        else {
            return;
        };
        if evidence.size != written {
            return;
        }
        self.stages.record(StageEvidence {
            stage: evidence,
            destination: Some(self.destination.clone()),
            existed: destination.and_then(presence),
        });
    }
}

/// The publication compound: CREATE(stage, not a folder, a link as itself)
/// + SET_INFO(rename without replacing) + CLOSE. The CREATE's attributes
/// describe the published file. A failure after a successful CREATE closes
/// the handle by hand, as `wire::create_set_close`.
pub(super) async fn rename_no_replace_evidenced(
    conn: &Connection,
    tree: &Tree,
    from: &str,
    to: &str,
) -> smb2::Result<Attributes> {
    let create = wire::open_request(
        tree,
        from,
        FileAccessMask::DELETE | FileAccessMask::FILE_READ_ATTRIBUTES,
        wire::FILE_OPEN_REPARSE_POINT | wire::FILE_NON_DIRECTORY_FILE,
    );
    let set = wire::set_file_info(
        FILE_RENAME_INFORMATION,
        FileId::SENTINEL,
        rename_info(&smb2::encode_path(to), false),
    );
    let close = wire::related_close();
    let ops = [
        CompoundOp::new(Command::Create, &create, Some(tree.tree_id)),
        CompoundOp::new(Command::SetInfo, &set, Some(tree.tree_id)),
        CompoundOp::new(Command::Close, &close, Some(tree.tree_id)),
    ];
    let frames = wire::frames(conn.execute_compound(&ops).await?, ops.len())?;
    let response = wire::created(&frames[0])?;
    let set_status = frames[1].header.status;
    if set_status != NtStatus::SUCCESS {
        wire::close_quietly(conn, tree, response.file_id).await;
        return Err(wire::protocol(set_status, Command::SetInfo));
    }
    if frames[2].header.status != NtStatus::SUCCESS {
        wire::close_quietly(conn, tree, response.file_id).await;
    }
    Ok(wire::attributes_of(&response))
}

#[cfg(test)]
mod tests {
    use super::{presence, regular_evidence, Attributes};

    const REGULAR: u32 = 0x20;
    const DIRECTORY: u32 = 0x10;
    const REPARSE: u32 = 0x400;

    fn attributes(attributes: u32, size: u64) -> Attributes {
        Attributes {
            attributes,
            size,
            last_write: 133_000_000_000_000_000,
            creation: 0,
            reparse_tag: None,
        }
    }

    #[test]
    fn speed_task_smb_evidence_only_for_regular_files() {
        let evidence = regular_evidence("/s/up/a.bin", &attributes(REGULAR, 5)).expect("file");
        assert_eq!((evidence.size, evidence.object.as_deref()), (5, None));
        assert!(evidence.mtime_ms > 0);
        assert!(regular_evidence("/s/up/a", &attributes(DIRECTORY, 0)).is_none());
        // Any reparse point, a data one included, is no evidence.
        assert!(regular_evidence("/s/up/a", &attributes(REGULAR | REPARSE, 5)).is_none());
    }

    #[test]
    fn speed_task_smb_presence_needs_a_deciding_answer() {
        assert_eq!(presence(&Ok(attributes(REGULAR, 1))), Some(true));
        let lost = smb2::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_eq!(presence(&Err(lost)), None);
    }
}
