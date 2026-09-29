//! Transfer-engine task tests for the SMB backend's pure parts: the runtime
//! size, the error meaning of the one-round-trip folder CREATE and the READ
//! chunk. Live SMB transfers run in the Android device suite against Samba.
use super::errors::map;
use super::session::runtime_workers;
use smb2::types::status::NtStatus;
use smb2::types::Command;
use smb2::Error;
use std::io;

#[test]
fn transfer_engine_task_smb_runtime_has_a_worker_per_core() {
    let cores = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    let workers = runtime_workers();
    assert!(workers >= 2, "the receiver task keeps a worker of its own");
    assert!(workers >= cores);
}

#[test]
fn transfer_engine_task_smb_folder_create_refusals_keep_their_meaning() {
    // `create_dir_new`: a taken name (file or folder) is `AlreadyExists`.
    let taken = Error::Protocol {
        status: NtStatus::OBJECT_NAME_COLLISION,
        command: Command::Create,
    };
    assert_eq!(
        map(taken, "Ordner anlegen", "/share/a").kind(),
        io::ErrorKind::AlreadyExists
    );
    // A missing parent is not a taken name.
    let missing = Error::Protocol {
        status: NtStatus::OBJECT_PATH_NOT_FOUND,
        command: Command::Create,
    };
    assert_eq!(
        map(missing, "Ordner anlegen", "/share/a/b").kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn transfer_engine_task_smb_overload_is_congestion() {
    for status in [
        NtStatus::INSUFFICIENT_RESOURCES,
        NtStatus::INSUFF_SERVER_RESOURCES,
        NtStatus::REQUEST_NOT_ACCEPTED,
    ] {
        let busy = Error::Protocol {
            status,
            command: Command::Read,
        };
        let mapped = map(busy, "Lesen", "/share/a");
        assert!(crate::vfs::congestion_of(&mapped).is_some(), "{mapped}");
    }
    let denied = Error::Protocol {
        status: NtStatus::ACCESS_DENIED,
        command: Command::Read,
    };
    assert!(crate::vfs::congestion_of(&map(denied, "Lesen", "/share/a")).is_none());
}

#[test]
fn transfer_engine_task_smb_read_chunk_is_smb2_download_chunk() {
    // 512 KiB READs: a slow link still delivers one about every 1.4 s
    // (smb2 read_ahead.rs), a fast one reaches its depth in a few round trips.
    assert_eq!(smb2::DOWNLOAD_CHUNK_SIZE, 512 * 1024);
}
