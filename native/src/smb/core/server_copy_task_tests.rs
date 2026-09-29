//! Transfer-engine task tests for the SMB server-side copy and the unsynced
//! stage commit: the CREATE and QUERY_INFO requests the copy sends, and the
//! whole copy flow against a scripted share. The resume-key and COPYCHUNK
//! wire format is smb2's own (tested in its copy.rs against a mock
//! transport; its connection test hooks are not public).
use super::errors::map;
use super::server_copy::{
    as_new_file, copy_to_stage, declined, end_of_file, length_request, refused, CopyOps,
    ServerCopy, SOURCE_ACCESS, STAGE_ACCESS,
};
use super::streams::unsynced_commit;
use super::wire::FILE_NON_DIRECTORY_FILE;
use smb2::msg::create::{CreateDisposition, CreateRequest, ImpersonationLevel, ShareAccess};
use smb2::msg::query_info::InfoType;
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, FileId, OplockLevel};
use smb2::Error;
use std::io;

const SOURCE: &str = "dir/source.bin";
const STAGE: &str = "dir/copy.bin.se-upload-1";

fn status(status: NtStatus, command: Command) -> Error {
    Error::Protocol { status, command }
}

/// A share with one source file; file 1 is the source, file 2 the stage.
struct Share {
    source: Option<u64>,
    /// Bytes a writer appends to the source while the server copies.
    grows: u64,
    /// The copy's answer instead of copying.
    copy_error: Option<NtStatus>,
    stage: Option<u64>,
    log: Vec<String>,
}

impl Share {
    fn new(source: Option<u64>) -> Self {
        Self {
            source,
            grows: 0,
            copy_error: None,
            stage: None,
            log: Vec::new(),
        }
    }

    fn run(&mut self, size: u64) -> smb2::Result<ServerCopy> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        runtime.block_on(copy_to_stage(self, SOURCE, STAGE, size))
    }
}

impl CopyOps for Share {
    type File = u8;

    async fn open_source(&mut self, rel: &str) -> smb2::Result<(u8, u64)> {
        self.log.push(format!("open {rel}"));
        let length = self
            .source
            .ok_or_else(|| status(NtStatus::OBJECT_NAME_NOT_FOUND, Command::Create))?;
        Ok((1, length))
    }

    async fn create_stage(&mut self, rel: &str) -> smb2::Result<u8> {
        self.log.push(format!("create {rel}"));
        if self.stage.is_some() {
            return Err(status(NtStatus::OBJECT_NAME_COLLISION, Command::Create));
        }
        self.stage = Some(0);
        Ok(2)
    }

    async fn copy(&mut self, source: u8, stage: u8, length: u64) -> smb2::Result<u64> {
        self.log.push(format!("copy {source}->{stage} {length}"));
        if let Some(code) = self.copy_error {
            return Err(status(code, Command::Ioctl));
        }
        self.stage = Some(length);
        let grows = self.grows;
        self.source = self.source.map(|source| source + grows);
        Ok(length)
    }

    async fn length(&mut self, file: u8) -> smb2::Result<u64> {
        self.log.push(format!("length {file}"));
        let length = if file == 1 { self.source } else { self.stage };
        Ok(length.unwrap_or(0))
    }

    async fn close(&mut self, file: u8) -> smb2::Result<()> {
        self.log.push(format!("close {file}"));
        Ok(())
    }

    async fn delete(&mut self, rel: &str) -> smb2::Result<()> {
        self.log.push(format!("delete {rel}"));
        self.stage = None;
        Ok(())
    }
}

#[test]
fn transfer_engine_task_smb_server_copy_requests_fit_copychunk() {
    assert_eq!(
        SOURCE_ACCESS,
        FileAccessMask::FILE_READ_DATA
            | FileAccessMask::FILE_READ_ATTRIBUTES
            | FileAccessMask::SYNCHRONIZE
    );
    let read_write = FileAccessMask::FILE_READ_DATA | FileAccessMask::FILE_WRITE_DATA;
    assert_eq!(
        STAGE_ACCESS & read_write,
        read_write,
        "COPYCHUNK needs read and write access on its destination"
    );
    // An open as wire.rs builds it becomes an exclusive create; nothing else
    // of it changes.
    let open = CreateRequest {
        requested_oplock_level: OplockLevel::None,
        impersonation_level: ImpersonationLevel::Impersonation,
        desired_access: FileAccessMask::new(STAGE_ACCESS),
        file_attributes: 0,
        share_access: ShareAccess(ShareAccess::FILE_SHARE_READ),
        create_disposition: CreateDisposition::FileOpen,
        create_options: FILE_NON_DIRECTORY_FILE,
        name: smb2::encode_path(STAGE),
        create_contexts: Vec::new(),
    };
    let expected = CreateRequest {
        create_disposition: CreateDisposition::FileCreate,
        file_attributes: 0x80,
        ..open.clone()
    };
    assert_eq!(as_new_file(open), expected);
    let query = length_request(FileId {
        persistent: 1,
        volatile: 2,
    });
    assert_eq!(query.info_type, InfoType::File);
    assert_eq!(query.file_info_class, 5, "FileStandardInformation");
    assert_eq!(query.output_buffer_length, 24);
}

#[test]
fn transfer_engine_task_smb_server_copy_reads_the_end_of_file() {
    let mut buffer = vec![0u8; 24];
    buffer[8..16].copy_from_slice(&1234u64.to_le_bytes());
    assert_eq!(end_of_file(&buffer).ok(), Some(1234));
    assert!(end_of_file(&buffer[..12]).is_err());
}

#[test]
fn transfer_engine_task_smb_server_copy_copies_and_closes_without_flush() {
    let mut share = Share::new(Some(1000));
    let copied = share.run(1000).ok();
    assert_eq!(copied, Some(ServerCopy::Copied(1000)));
    assert_eq!(copied.and_then(|copy| copy.engine_answer()), Some(1000));
    assert_eq!(
        share.log,
        [
            "open dir/source.bin",
            "create dir/copy.bin.se-upload-1",
            "copy 1->2 1000",
            "length 1",
            "close 1",
            "close 2",
        ]
    );
    assert_eq!(share.stage, Some(1000));
    let mut empty = Share::new(Some(0));
    assert_eq!(empty.run(0).ok(), Some(ServerCopy::Copied(0)));
}

#[test]
fn transfer_engine_task_smb_server_copy_shows_changed_sources() {
    // Changed since the listing: nothing is created.
    let mut shrunk = Share::new(Some(990));
    assert_eq!(shrunk.run(1000).ok(), Some(ServerCopy::Changed(990)));
    assert_eq!(shrunk.log, ["open dir/source.bin", "close 1"]);
    assert_eq!(shrunk.stage, None);
    // Appended while the server copied: the engine discards the stage.
    let mut grown = Share::new(Some(1000));
    grown.grows = 10;
    let copied = grown.run(1000).ok();
    assert_eq!(copied, Some(ServerCopy::Changed(1010)));
    assert_eq!(copied.and_then(|copy| copy.engine_answer()), Some(1010));
    assert!(!grown.log.iter().any(|entry| entry.starts_with("delete")));
}

#[test]
fn transfer_engine_task_smb_server_copy_unsupported_falls_back_to_streaming() {
    for code in [
        NtStatus::NOT_SUPPORTED,
        NtStatus::INVALID_DEVICE_REQUEST,
        NtStatus::NOT_IMPLEMENTED,
    ] {
        let mut share = Share::new(Some(1000));
        share.copy_error = Some(code);
        let copied = share.run(1000).ok();
        assert_eq!(copied, Some(ServerCopy::Refused));
        assert_eq!(copied.and_then(|copy| copy.engine_answer()), None);
        assert_eq!(
            share.log[3..],
            ["close 1", "close 2", "delete dir/copy.bin.se-upload-1"]
        );
        assert_eq!(share.stage, None, "nothing of the attempt stays");
    }
    assert!(!refused(&status(NtStatus::ACCESS_DENIED, Command::Ioctl)));
}

#[test]
fn transfer_engine_task_smb_server_copy_declined_copy_streams() {
    // Any other answer to the copy requests: same-share copies streamed
    // before, so they keep streaming instead of failing.
    for code in [NtStatus::ACCESS_DENIED, NtStatus::INVALID_PARAMETER] {
        let mut share = Share::new(Some(1000));
        share.copy_error = Some(code);
        assert_eq!(share.run(1000).ok(), Some(ServerCopy::Stream));
        assert_eq!(
            share.log[3..],
            ["close 1", "close 2", "delete dir/copy.bin.se-upload-1"]
        );
        assert_eq!(share.stage, None);
    }
    let odd = Error::invalid_data("server rejected server-side copy");
    assert_eq!(declined(&odd), Some(ServerCopy::Stream));
    // A full target, an overloaded server or a lost connection is no
    // answer about the copy.
    assert_eq!(declined(&status(NtStatus::DISK_FULL, Command::Ioctl)), None);
    let busy = status(NtStatus::INSUFFICIENT_RESOURCES, Command::Ioctl);
    assert_eq!(declined(&busy), None);
    let mapped = map(busy, "Kopieren", STAGE);
    assert!(crate::vfs::congestion_of(&mapped).is_some(), "{mapped}");
    assert_eq!(declined(&Error::Disconnected), None);
}

#[test]
fn transfer_engine_task_smb_server_copy_missing_source_streams() {
    let mut share = Share::new(None);
    let copied = share.run(10).ok();
    assert_eq!(copied, Some(ServerCopy::Stream));
    assert_eq!(copied.and_then(|copy| copy.engine_answer()), None);
    assert_eq!(share.log, ["open dir/source.bin"]);
}

#[test]
fn transfer_engine_task_smb_server_copy_taken_stage_is_already_exists() {
    let mut share = Share::new(Some(10));
    share.stage = Some(3);
    let error = share
        .run(10)
        .err()
        .map(|error| map(error, "Kopieren", STAGE));
    assert_eq!(
        error.map(|error| error.kind()),
        Some(io::ErrorKind::AlreadyExists)
    );
    assert_eq!(share.stage, Some(3), "a foreign file stays untouched");
    assert_eq!(
        share.log,
        [
            "open dir/source.bin",
            "create dir/copy.bin.se-upload-1",
            "close 1"
        ]
    );
}

#[test]
fn transfer_engine_task_smb_server_copy_failure_leaves_the_stage_to_the_engine() {
    let mut share = Share::new(Some(10));
    share.copy_error = Some(NtStatus::DISK_FULL);
    let error = share
        .run(10)
        .err()
        .map(|error| map(error, "Kopieren", STAGE));
    assert_eq!(
        error.map(|error| error.kind()),
        Some(io::ErrorKind::StorageFull)
    );
    assert_eq!(share.log[3..], ["close 1", "close 2"]);
    assert_eq!(share.stage, Some(0));
}

#[test]
fn transfer_engine_task_smb_unsynced_commit_needs_every_byte_confirmed() {
    assert!(unsynced_commit(10, 10).is_ok());
    assert!(unsynced_commit(0, 0).is_ok());
    assert!(unsynced_commit(10, 9).is_err());
}
