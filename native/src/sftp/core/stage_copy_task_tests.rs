//! Transfer-engine task tests for the unsynced stage commit and the
//! server-side copy (`copy-data`). The server is russh-sftp's own server
//! loop over an in-memory stream with a scripted handler that answers like
//! OpenSSH's `process_extended_copy_data`, so requests cross the real wire
//! encoding in both directions.
use super::channel_pool::offered;
use super::copy_data::{
    copy_data_payload, copy_to_stage, next_range, range_answer, RangeAnswer, ServerCopy, COPY_DATA,
    FIRST_RANGE, MIN_RANGE,
};
use super::io_adapters::Commit;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::RawSftpSession;
use russh_sftp::protocol::{
    Attrs, FileAttributes, Handle, OpenFlags, Packet, Status, StatusCode, Version,
};
use russh_sftp::server::Handler;
use std::collections::HashMap;
use std::future::{ready, Future};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

const SOURCE: &str = "/data/source.bin";
const STAGE: &str = "/data/copy.bin.se-upload-00ff";
/// sftp-server.c copies in steps of its 64 KiB buffer.
const OPENSSH_STEP: u64 = 64 * 1024;

#[derive(Default)]
struct Disk {
    source: Option<u64>,
    stage: Option<u64>,
    /// (read offset, length, write offset) of every copy-data request.
    ranges: Vec<(u64, u64, u64)>,
    /// Other extended requests (fsync would show here).
    others: Vec<String>,
    /// FSTAT answers FAILURE (a step after the copy that fails).
    fstat_fails: bool,
    opened: Vec<String>,
    closed: Vec<String>,
    removed: Vec<String>,
}

#[derive(Clone)]
struct FakeServer {
    /// `copy-data` "1" in the VERSION reply; without it the request is
    /// unknown (`OP_UNSUPPORTED`).
    offer: bool,
    /// Answer every copy-data with this status instead of copying.
    answer: Option<StatusCode>,
    disk: Arc<Mutex<Disk>>,
}

impl FakeServer {
    fn new(offer: bool, answer: Option<StatusCode>, source: Option<u64>) -> Self {
        let disk = Disk {
            source,
            ..Disk::default()
        };
        Self {
            offer,
            answer,
            disk: Arc::new(Mutex::new(disk)),
        }
    }

    fn disk(&self) -> MutexGuard<'_, Disk> {
        self.disk.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn copy_data(&self, id: u32, data: &[u8]) -> Result<Packet, StatusCode> {
        let (read, read_offset, length, write, write_offset) =
            parse_copy(data).ok_or(StatusCode::BadMessage)?;
        let mut disk = self.disk();
        disk.ranges.push((read_offset, length, write_offset));
        if !self.offer {
            return Err(StatusCode::OpUnsupported);
        }
        if let Some(code) = self.answer {
            return Ok(status(id, code));
        }
        let source = disk.source.ok_or(StatusCode::Failure)?;
        if read != "r" || write != "w" {
            return Err(StatusCode::Failure);
        }
        let available = source.saturating_sub(read_offset);
        let copied = if length == 0 {
            available
        } else {
            length.min(available)
        };
        if copied > 0 {
            let end = write_offset + copied;
            disk.stage = Some(disk.stage.unwrap_or(0).max(end));
        }
        Ok(status(id, openssh_status(length, available)))
    }
}

/// sftp-server.c: `read_len` shrinks before each 64 KiB read, so a fixed
/// length that ends within the source's last step answers OK; an earlier
/// end answers EOF. Length 0 (until EOF) always answers OK.
fn openssh_status(length: u64, available: u64) -> StatusCode {
    if length == 0 {
        return StatusCode::Ok;
    }
    let last_step = match length % OPENSSH_STEP {
        0 => OPENSSH_STEP,
        rest => rest,
    };
    if available >= length - last_step {
        StatusCode::Ok
    } else {
        StatusCode::Eof
    }
}

fn status(id: u32, status_code: StatusCode) -> Packet {
    Packet::Status(Status {
        id,
        status_code,
        error_message: status_code.to_string(),
        language_tag: "en-US".to_string(),
    })
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en-US".to_string(),
    }
}

fn take_string(data: &mut &[u8]) -> Option<String> {
    let (length, rest) = data.split_at_checked(4)?;
    let length = u32::from_be_bytes(length.try_into().ok()?) as usize;
    let (bytes, rest) = rest.split_at_checked(length)?;
    *data = rest;
    String::from_utf8(bytes.to_vec()).ok()
}

fn take_u64(data: &mut &[u8]) -> Option<u64> {
    let (bytes, rest) = data.split_at_checked(8)?;
    *data = rest;
    Some(u64::from_be_bytes(bytes.try_into().ok()?))
}

/// Decodes a copy-data body in the order sftp-server.c reads it.
fn parse_copy(mut data: &[u8]) -> Option<(String, u64, u64, String, u64)> {
    let parsed = (
        take_string(&mut data)?,
        take_u64(&mut data)?,
        take_u64(&mut data)?,
        take_string(&mut data)?,
        take_u64(&mut data)?,
    );
    data.is_empty().then_some(parsed)
}

impl Handler for FakeServer {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> impl Future<Output = Result<Version, Self::Error>> + Send {
        let mut version = Version::new();
        if self.offer {
            version
                .extensions
                .insert(COPY_DATA.to_string(), "1".to_string());
        }
        ready(Ok(version))
    }

    fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> impl Future<Output = Result<Handle, Self::Error>> + Send {
        let mut disk = self.disk();
        disk.opened.push(filename.clone());
        let exclusive = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE;
        let handle = if filename == SOURCE && pflags.bits() == OpenFlags::READ.bits() {
            disk.source.map(|_| "r").ok_or(StatusCode::NoSuchFile)
        } else if filename == STAGE && pflags.bits() == exclusive.bits() {
            // OpenSSH answers EEXIST with a plain FAILURE in protocol 3.
            match disk.stage {
                Some(_) => Err(StatusCode::Failure),
                None => {
                    disk.stage = Some(0);
                    Ok("w")
                }
            }
        } else {
            Err(StatusCode::NoSuchFile)
        };
        ready(handle.map(|handle| Handle {
            id,
            handle: handle.to_string(),
        }))
    }

    fn close(
        &mut self,
        id: u32,
        handle: String,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        self.disk().closed.push(handle);
        ready(Ok(ok(id)))
    }

    fn fstat(
        &mut self,
        id: u32,
        handle: String,
    ) -> impl Future<Output = Result<Attrs, Self::Error>> + Send {
        let disk = self.disk();
        if disk.fstat_fails {
            return ready(Err(StatusCode::Failure));
        }
        let size = match handle.as_str() {
            "w" => disk.stage,
            _ => disk.source,
        };
        ready(Ok(Attrs {
            id,
            attrs: FileAttributes {
                size,
                ..FileAttributes::empty()
            },
        }))
    }

    fn lstat(
        &mut self,
        id: u32,
        path: String,
    ) -> impl Future<Output = Result<Attrs, Self::Error>> + Send {
        let size = match path.as_str() {
            STAGE => self.disk().stage,
            SOURCE => self.disk().source,
            _ => None,
        };
        ready(size.ok_or(StatusCode::NoSuchFile).map(|size| Attrs {
            id,
            attrs: FileAttributes {
                size: Some(size),
                ..FileAttributes::empty()
            },
        }))
    }

    fn remove(
        &mut self,
        id: u32,
        filename: String,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        let mut disk = self.disk();
        if filename == STAGE {
            disk.stage = None;
        }
        disk.removed.push(filename);
        ready(Ok(ok(id)))
    }

    fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<Packet, Self::Error>> + Send {
        if request == COPY_DATA {
            return ready(self.copy_data(id, &data));
        }
        self.disk().others.push(request);
        ready(Err(StatusCode::OpUnsupported))
    }
}

/// A client session on `server` after INIT (the client refuses answers
/// before the VERSION reply), with that reply.
fn connect(server: &FakeServer) -> (tokio::runtime::Runtime, Arc<RawSftpSession>, Version) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("tokio runtime");
    let (client, served) = tokio::io::duplex(1 << 20);
    let handler = server.clone();
    let (session, version) = runtime.block_on(async move {
        russh_sftp::server::run(served, handler).await;
        let session = RawSftpSession::new(client);
        let version = session.init().await.expect("VERSION");
        (session, version)
    });
    (runtime, Arc::new(session), version)
}

fn copy(server: &FakeServer, size: u64) -> io::Result<ServerCopy> {
    let (runtime, session, _) = connect(server);
    runtime.block_on(copy_to_stage(
        &session,
        SOURCE,
        STAGE,
        size,
        &std::sync::atomic::AtomicBool::new(false),
        &|_: &SftpError| {},
    ))
}

#[test]
fn transfer_engine_task_sftp_unsynced_stage_never_syncs() {
    assert!(Commit::Durable.syncs(true));
    assert!(!Commit::Durable.syncs(false));
    assert!(!Commit::Unsynced.syncs(true));
    assert!(!Commit::Unsynced.syncs(false));
}

#[test]
fn transfer_engine_task_sftp_copy_data_payload_matches_protocol() {
    let payload = copy_data_payload("ab", 1, 0x0102, "xyz", 0x0304_0506_0708_090a);
    let mut expected = vec![0, 0, 0, 2, b'a', b'b'];
    expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 2]);
    expected.extend_from_slice(&[0, 0, 0, 3, b'x', b'y', b'z']);
    expected.extend_from_slice(&[3, 4, 5, 6, 7, 8, 9, 10]);
    assert_eq!(payload, expected);
    assert_eq!(
        parse_copy(&payload),
        Some(("ab".into(), 1, 0x0102, "xyz".into(), 0x0304_0506_0708_090a))
    );
}

#[test]
fn transfer_engine_task_sftp_copy_data_offered_only_in_version_1() {
    let mut version = Version::new();
    assert!(!offered(&version, COPY_DATA));
    version.extensions.insert(COPY_DATA.into(), "2".into());
    assert!(!offered(&version, COPY_DATA));
    version.extensions.insert(COPY_DATA.into(), "1".into());
    assert!(offered(&version, COPY_DATA));
    for offer in [true, false] {
        let (_, _, version) = connect(&FakeServer::new(offer, None, Some(1)));
        assert_eq!(offered(&version, COPY_DATA), offer);
    }
}

#[test]
fn transfer_engine_task_sftp_copy_data_ranges_follow_the_rate() {
    // 16 MiB in 2 s is 8 MiB/s: the next range lasts 10 s.
    assert_eq!(next_range(FIRST_RANGE, Duration::from_secs(2)), 80 << 20);
    assert_eq!(
        next_range(FIRST_RANGE, Duration::from_secs(100_000)),
        MIN_RANGE
    );
    assert!(next_range(FIRST_RANGE, Duration::ZERO) > FIRST_RANGE);
}

#[test]
fn transfer_engine_task_sftp_copy_data_answers_map_to_fallbacks() {
    let answer = |code| range_answer(&status(1, code));
    assert_eq!(answer(StatusCode::Ok), RangeAnswer::Copied);
    assert_eq!(answer(StatusCode::Eof), RangeAnswer::SourceEnded);
    assert_eq!(answer(StatusCode::OpUnsupported), RangeAnswer::Refused);
    assert_eq!(answer(StatusCode::PermissionDenied), RangeAnswer::Refused);
    for code in [
        StatusCode::Failure,
        StatusCode::NoSuchFile,
        StatusCode::BadMessage,
    ] {
        assert_eq!(answer(code), RangeAnswer::Declined);
    }
    let handle = Packet::Handle(Handle {
        id: 1,
        handle: "x".to_string(),
    });
    assert_eq!(range_answer(&handle), RangeAnswer::Declined);
}

#[test]
fn transfer_engine_task_sftp_server_copy_runs_in_ranges_without_sync() {
    let size = FIRST_RANGE + (4 << 20) + 5;
    let server = FakeServer::new(true, None, Some(size));
    assert_eq!(copy(&server, size).ok(), Some(ServerCopy::Copied(size)));
    let disk = server.disk();
    // The last range asks one byte beyond the size, so growth would show.
    assert_eq!(
        disk.ranges,
        vec![
            (0, FIRST_RANGE, 0),
            (FIRST_RANGE, size + 1 - FIRST_RANGE, FIRST_RANGE)
        ]
    );
    assert!(disk.others.is_empty(), "no fsync: {:?}", disk.others);
    assert_eq!(disk.closed, vec!["r", "w"]);
    assert!(disk.removed.is_empty());
    assert_eq!(disk.stage, Some(size));
}

#[test]
fn transfer_engine_task_sftp_server_copy_shows_changed_sources() {
    let size = FIRST_RANGE + (4 << 20) + 5;
    // Grown, shrunk inside the first range (OK, then EOF on the next),
    // shrunk inside the last step (OK), and an empty file.
    for (expected, source, copied, ranges) in [
        (size, size + 100, size + 1, 2),
        (size, FIRST_RANGE - 3, FIRST_RANGE - 3, 2),
        (size, size - 2, size - 2, 2),
        (0, 0, 0, 1),
    ] {
        let server = FakeServer::new(true, None, Some(source));
        assert_eq!(
            copy(&server, expected).ok(),
            Some(ServerCopy::Copied(copied)),
            "source {source}"
        );
        assert_eq!(server.disk().ranges.len(), ranges, "source {source}");
    }
}

#[test]
fn transfer_engine_task_sftp_server_copy_refusal_removes_the_stage() {
    let cases = [
        (true, Some(StatusCode::OpUnsupported)),
        (true, Some(StatusCode::PermissionDenied)),
        (false, None),
    ];
    for (offer, answer) in cases {
        let server = FakeServer::new(offer, answer, Some(1000));
        assert_eq!(copy(&server, 1000).ok(), Some(ServerCopy::Refused));
        let disk = server.disk();
        assert_eq!(disk.closed, vec!["r", "w"]);
        assert_eq!(disk.removed, vec![STAGE]);
        assert_eq!(disk.stage, None);
    }
}

#[test]
fn transfer_engine_task_sftp_server_copy_missing_source_creates_nothing() {
    let server = FakeServer::new(true, None, None);
    assert_eq!(copy(&server, 10).ok(), Some(ServerCopy::Stream));
    let disk = server.disk();
    assert_eq!(disk.opened, vec![SOURCE]);
    assert_eq!(disk.stage, None);
    assert!(disk.ranges.is_empty());
}

#[test]
fn transfer_engine_task_sftp_server_copy_taken_stage_is_already_exists() {
    let server = FakeServer::new(true, None, Some(10));
    server.disk().stage = Some(3);
    let error = copy(&server, 10).err().map(|error| error.kind());
    assert_eq!(error, Some(io::ErrorKind::AlreadyExists));
    let disk = server.disk();
    assert_eq!(disk.stage, Some(3), "a foreign file stays untouched");
    assert!(disk.removed.is_empty());
    assert_eq!(disk.closed, vec!["r"]);
}

#[test]
fn transfer_engine_task_sftp_server_copy_declined_copy_streams() {
    // Copies inside one server streamed before; a server that answers the
    // copy with another status keeps them streaming instead of failing.
    for code in [StatusCode::Failure, StatusCode::BadMessage] {
        let server = FakeServer::new(true, Some(code), Some(10));
        assert_eq!(copy(&server, 10).ok(), Some(ServerCopy::Stream));
        let disk = server.disk();
        assert_eq!(disk.closed, vec!["r", "w"]);
        assert_eq!(disk.removed, vec![STAGE]);
        assert_eq!(disk.stage, None);
    }
}

#[test]
fn transfer_engine_task_sftp_server_copy_failure_after_the_copy_is_an_error() {
    let server = FakeServer::new(true, None, Some(10));
    server.disk().fstat_fails = true;
    let error = copy(&server, 10).err().map(|error| error.kind());
    assert!(error.is_some_and(|kind| kind != io::ErrorKind::AlreadyExists));
    let disk = server.disk();
    assert_eq!(disk.closed, vec!["r", "w"]);
    // The engine discards the stage it named.
    assert!(disk.removed.is_empty());
    assert_eq!(disk.stage, Some(10));
}
