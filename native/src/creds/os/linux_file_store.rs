use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const LOCK_NAME: &str = "store.lock";
const MAGIC: &[u8; 8] = b"SESEC01\0";
const FORMAT_VERSION: u8 = 1;
const ACCOUNT_DIGEST_BYTES: usize = 32;
const CHECKSUM_BYTES: usize = 32;
const HEADER_BYTES: usize = MAGIC.len() + 1 + ACCOUNT_DIGEST_BYTES + 4;
const MIN_RECORD_BYTES: usize = HEADER_BYTES + CHECKSUM_BYTES;
const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_RECORD_BYTES: usize = HEADER_BYTES + MAX_SECRET_BYTES + CHECKSUM_BYTES;
const STAGE_ATTEMPTS: usize = 64;

pub(super) struct FileStore {
    directory: PathBuf,
}

struct StoreLock {
    _file: File,
}

impl FileStore {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub(super) fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        validate_account(account)?;
        if secret.len() > MAX_SECRET_BYTES {
            return Err(format!(
                "Linux credential store: secret exceeds {MAX_SECRET_BYTES} bytes"
            ));
        }

        let directory = self.open_directory()?;
        let _lock = acquire_lock(&directory)?;
        let final_name = record_name(account);
        validate_existing_record(&directory, &final_name)?;
        let record = encode_record(account, secret);
        let (stage_name, mut stage) = create_stage(&directory)?;
        let mut promoted = false;
        let result = (|| {
            stage
                .write_all(&record)
                .map_err(|error| io_error("write staged record", error))?;
            stage
                .sync_all()
                .map_err(|error| io_error("sync staged record", error))?;
            validate_record_file(&stage, "staged record")?;
            drop(stage);
            rename_in(&directory, &stage_name, &final_name)?;
            promoted = true;
            directory
                .sync_all()
                .map_err(|error| io_error("sync credential directory", error))
        })();
        if result.is_err() && !promoted {
            let _ = unlink_in(&directory, &stage_name);
        }
        result
    }

    pub(super) fn get(&self, account: &str) -> Result<Option<String>, String> {
        validate_account(account)?;
        let directory = self.open_directory()?;
        let _lock = acquire_lock(&directory)?;
        let name = record_name(account);
        let Some(file) = open_record(&directory, &name)? else {
            return Ok(None);
        };
        let size = validate_record_file(&file, "credential record")?;
        if size > MAX_RECORD_BYTES {
            return Err(format!(
                "Linux credential store: record exceeds {MAX_RECORD_BYTES} bytes"
            ));
        }
        let mut bytes = Vec::with_capacity(size);
        file.take((MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| io_error("read credential record", error))?;
        decode_record(account, &bytes).map(Some)
    }

    pub(super) fn delete(&self, account: &str) -> Result<(), String> {
        validate_account(account)?;
        let directory = self.open_directory()?;
        let _lock = acquire_lock(&directory)?;
        let name = record_name(account);
        let Some(file) = open_record(&directory, &name)? else {
            return Ok(());
        };
        validate_record_file(&file, "credential record")?;
        drop(file);
        unlink_in(&directory, &name)?;
        directory
            .sync_all()
            .map_err(|error| io_error("sync credential directory", error))
    }

    fn open_directory(&self) -> Result<File, String> {
        let created = match DirBuilder::new()
            .mode(DIRECTORY_MODE)
            .create(&self.directory)
        {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(io_error("create credential directory", error)),
        };
        if created {
            std::fs::set_permissions(
                &self.directory,
                std::fs::Permissions::from_mode(DIRECTORY_MODE),
            )
            .map_err(|error| io_error("set credential directory permissions", error))?;
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.directory)
            .map_err(|error| io_error("open credential directory", error))?;
        validate_directory(&directory)?;
        Ok(directory)
    }

    #[cfg(test)]
    fn record_path(&self, account: &str) -> PathBuf {
        self.directory.join(record_name(account))
    }
}

fn validate_account(account: &str) -> Result<(), String> {
    if account.is_empty() {
        Err("Linux credential store: account must not be empty".into())
    } else {
        Ok(())
    }
}

fn validate_directory(directory: &File) -> Result<(), String> {
    let metadata = directory
        .metadata()
        .map_err(|error| io_error("inspect credential directory", error))?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != DIRECTORY_MODE
    {
        return Err(
            "Linux credential store: directory must be user-owned mode 0700 and not a link".into(),
        );
    }
    Ok(())
}

fn acquire_lock(directory: &File) -> Result<StoreLock, String> {
    let name = c_name(LOCK_NAME)?;
    let exclusive_flags = libc::O_RDWR
        | libc::O_CREAT
        | libc::O_EXCL
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK;
    let (fd, created) = match openat(directory.as_raw_fd(), &name, exclusive_flags, FILE_MODE) {
        Ok(fd) => (fd, true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (
            openat(
                directory.as_raw_fd(),
                &name,
                libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0,
            )
            .map_err(|error| io_error("open credential lock", error))?,
            false,
        ),
        Err(error) => return Err(io_error("create credential lock", error)),
    };
    let file = unsafe { File::from_raw_fd(fd) };
    if created {
        file.set_permissions(std::fs::Permissions::from_mode(FILE_MODE))
            .map_err(|error| io_error("set credential lock permissions", error))?;
    }
    validate_secure_file(&file, "credential lock")?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io_error(
            "lock credential store",
            io::Error::last_os_error(),
        ));
    }
    validate_secure_file(&file, "credential lock")?;
    Ok(StoreLock { _file: file })
}

fn validate_existing_record(directory: &File, name: &str) -> Result<(), String> {
    if let Some(file) = open_record(directory, name)? {
        validate_record_file(&file, "existing credential record")?;
    }
    Ok(())
}

fn open_record(directory: &File, name: &str) -> Result<Option<File>, String> {
    let name = c_name(name)?;
    match openat(
        directory.as_raw_fd(),
        &name,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        0,
    ) {
        Ok(fd) => Ok(Some(unsafe { File::from_raw_fd(fd) })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("open credential record", error)),
    }
}

fn validate_record_file(file: &File, label: &str) -> Result<usize, String> {
    validate_secure_file(file, label)?;
    let length = file
        .metadata()
        .map_err(|error| io_error(&format!("inspect {label}"), error))?
        .len();
    usize::try_from(length)
        .map_err(|_| format!("Linux credential store: {label} length does not fit this platform"))
}

fn validate_secure_file(file: &File, label: &str) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|error| io_error(&format!("inspect {label}"), error))?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != FILE_MODE
        || metadata.nlink() != 1
    {
        return Err(format!(
            "Linux credential store: {label} must be a single-link, user-owned mode 0600 regular file"
        ));
    }
    Ok(())
}

fn create_stage(directory: &File) -> Result<(String, File), String> {
    for _ in 0..STAGE_ATTEMPTS {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce)
            .map_err(|error| format!("Linux credential store: generate staging nonce: {error}"))?;
        let name = format!(".stage-{}-{}", std::process::id(), hex(&nonce));
        let c_name = c_name(&name)?;
        match openat(
            directory.as_raw_fd(),
            &c_name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            FILE_MODE,
        ) {
            Ok(fd) => {
                let file = unsafe { File::from_raw_fd(fd) };
                let setup = file
                    .set_permissions(std::fs::Permissions::from_mode(FILE_MODE))
                    .map_err(|error| io_error("set staged record permissions", error))
                    .and_then(|()| validate_secure_file(&file, "staged record"));
                match setup {
                    Ok(()) => return Ok((name, file)),
                    Err(error) => {
                        drop(file);
                        let _ = unlink_in(directory, &name);
                        return Err(error);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error("create staged record", error)),
        }
    }
    Err("Linux credential store: could not allocate a unique staging file".into())
}

fn openat(directory: RawFd, name: &CString, flags: i32, mode: u32) -> io::Result<RawFd> {
    let fd = unsafe { libc::openat(directory, name.as_ptr(), flags, mode) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn rename_in(directory: &File, source: &str, destination: &str) -> Result<(), String> {
    let source = c_name(source)?;
    let destination = c_name(destination)?;
    if unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
        )
    } != 0
    {
        return Err(io_error(
            "promote credential record",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

fn unlink_in(directory: &File, name: &str) -> Result<(), String> {
    let name = c_name(name)?;
    if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::NotFound {
            return Err(io_error("remove credential record", error));
        }
    }
    Ok(())
}

fn encode_record(account: &str, secret: &str) -> Vec<u8> {
    let account_digest = account_digest(account);
    let mut record = Vec::with_capacity(HEADER_BYTES + secret.len() + CHECKSUM_BYTES);
    record.extend_from_slice(MAGIC);
    record.push(FORMAT_VERSION);
    record.extend_from_slice(&account_digest);
    record.extend_from_slice(&(secret.len() as u32).to_be_bytes());
    record.extend_from_slice(secret.as_bytes());
    let checksum = Sha256::digest(&record);
    record.extend_from_slice(&checksum);
    record
}

fn decode_record(account: &str, record: &[u8]) -> Result<String, String> {
    if record.len() < MIN_RECORD_BYTES {
        return Err("Linux credential store: truncated credential record".into());
    }
    if record.len() > MAX_RECORD_BYTES {
        return Err(format!(
            "Linux credential store: record exceeds {MAX_RECORD_BYTES} bytes"
        ));
    }
    if &record[..MAGIC.len()] != MAGIC {
        return Err("Linux credential store: invalid credential record magic".into());
    }
    if record[MAGIC.len()] != FORMAT_VERSION {
        return Err("Linux credential store: unsupported credential record version".into());
    }
    let digest_start = MAGIC.len() + 1;
    let digest_end = digest_start + ACCOUNT_DIGEST_BYTES;
    if record[digest_start..digest_end] != account_digest(account) {
        return Err("Linux credential store: credential record belongs to another account".into());
    }
    let length_end = digest_end + 4;
    let secret_length = u32::from_be_bytes(
        record[digest_end..length_end]
            .try_into()
            .map_err(|_| "Linux credential store: invalid secret length".to_string())?,
    ) as usize;
    if secret_length > MAX_SECRET_BYTES {
        return Err(format!(
            "Linux credential store: secret exceeds {MAX_SECRET_BYTES} bytes"
        ));
    }
    let expected = HEADER_BYTES
        .checked_add(secret_length)
        .and_then(|length| length.checked_add(CHECKSUM_BYTES))
        .ok_or_else(|| "Linux credential store: credential length overflow".to_string())?;
    if record.len() != expected {
        return Err("Linux credential store: credential record length mismatch".into());
    }
    let checksum_start = expected - CHECKSUM_BYTES;
    let expected_checksum = Sha256::digest(&record[..checksum_start]);
    if record[checksum_start..] != expected_checksum[..] {
        return Err("Linux credential store: credential record checksum mismatch".into());
    }
    String::from_utf8(record[HEADER_BYTES..checksum_start].to_vec())
        .map_err(|_| "Linux credential store: secret is not valid UTF-8".into())
}

fn record_name(account: &str) -> String {
    format!("{}.secret", hex(&account_digest(account)))
}

fn account_digest(account: &str) -> [u8; ACCOUNT_DIGEST_BYTES] {
    let mut hasher = Sha256::new();
    hasher.update(b"smart_explorer\0");
    hasher.update(account.as_bytes());
    hasher.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn c_name(name: &str) -> Result<CString, String> {
    CString::new(name)
        .map_err(|_| "Linux credential store: generated filename contains NUL".to_string())
}

fn io_error(action: &str, error: io::Error) -> String {
    format!("Linux credential store: {action}: {error}")
}

#[cfg(test)]
#[path = "linux_file_store_tests.rs"]
mod tests;
