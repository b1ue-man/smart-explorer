use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const IPC_ADDR_FILE: &str = "daemon.ipc";
const IPC_GENERATION_FILE: &str = "daemon.generation";
const IPC_TOKEN_FILE: &str = "daemon.token";
const EXEC_JOURNAL_FILE: &str = "exec-grants.journal";
const MAX_TOKEN_BYTES: u64 = 4 * 1024;

fn ipc_addr_path() -> io::Result<PathBuf> {
    Ok(sync_data_directory()?.join(IPC_ADDR_FILE))
}

fn ipc_generation_path() -> io::Result<PathBuf> {
    Ok(sync_data_directory()?.join(IPC_GENERATION_FILE))
}

pub(super) fn clear_ipc_addr() {
    if let Ok(path) = ipc_addr_path() {
        let _ = std::fs::remove_file(path);
    }
}

pub(super) fn clear_ipc_generation() {
    if let Ok(path) = ipc_generation_path() {
        let _ = std::fs::remove_file(path);
    }
}

pub(super) fn write_ipc_addr(addr: SocketAddr) -> io::Result<()> {
    std::fs::write(ipc_addr_path()?, addr.to_string())
}

pub(super) fn write_ipc_generation(generation: &str) -> io::Result<()> {
    std::fs::write(ipc_generation_path()?, generation)
}

pub(super) fn read_ipc_addr() -> Option<SocketAddr> {
    std::fs::read_to_string(ipc_addr_path().ok()?)
        .ok()
        .and_then(|text| text.trim().parse().ok())
}

pub(super) fn read_ipc_generation() -> Option<String> {
    let generation = std::fs::read_to_string(ipc_generation_path().ok()?).ok()?;
    let generation = generation.trim();
    (generation.len() == 32 && generation.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| generation.to_string())
}

pub(super) fn exec_journal_path() -> io::Result<PathBuf> {
    Ok(sync_data_directory()?.join(EXEC_JOURNAL_FILE))
}

pub(super) fn open_exec_journal() -> io::Result<File> {
    let file = open_file_without_following_links(&exec_journal_path()?)?;
    validate_private_file_handle(&file, "Exec grant journal")?;
    Ok(file)
}

pub(super) fn secure_exec_journal_temp(file: &File) -> io::Result<()> {
    validate_private_file_handle(file, "Exec grant journal temporary file")
}

pub(super) fn sync_exec_journal_directory() -> io::Result<()> {
    sync_data_directory().map(|_| ())
}

pub(super) fn commit_exec_journal_temp(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn load_or_create_token() -> io::Result<String> {
    let path = sync_data_directory()?.join(IPC_TOKEN_FILE);
    match read_token_path(&path) {
        Ok(token) => Ok(token),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_token(&path),
        Err(error) => Err(error),
    }
}

pub(super) fn read_token() -> io::Result<String> {
    read_token_path(&sync_data_directory()?.join(IPC_TOKEN_FILE))
}

fn sync_data_directory() -> io::Result<PathBuf> {
    let app = crate::support_dirs::app_data_dir();
    validate_directory(&app)?;
    let sync = app.join("sync");
    std::fs::create_dir_all(&sync)?;
    validate_directory(&sync)?;
    Ok(sync)
}

fn validate_directory(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata_is_link_like(&metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon IPC data path must be a directory, not a link",
        ));
    }
    Ok(())
}

fn create_token(path: &Path) -> io::Result<String> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return read_token_path(path);
        }
        Err(error) => return Err(error),
    };
    let result = (|| {
        let token = generate_token()?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        read_token_path(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn read_token_path(path: &Path) -> io::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon IPC token must be a regular file, not a link",
        ));
    }
    if metadata.len() > MAX_TOKEN_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon IPC token file too large",
        ));
    }
    let mut file = open_token_without_following_links(path)?;
    validate_token_handle(&file)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    let token = text.trim();
    if token.len() < 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon IPC token too short",
        ));
    }
    Ok(token.to_string())
}

fn metadata_is_link_like(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn open_token_without_following_links(path: &Path) -> io::Result<File> {
    open_file_without_following_links(path)
}

fn open_file_without_following_links(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn validate_token_handle(file: &File) -> io::Result<()> {
    validate_private_file_handle(file, "daemon IPC token")
}

fn validate_private_file_handle(file: &File, label: &str) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if information.nNumberOfLinks != 1
        || information.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{label} must be a single-link regular file"),
        ));
    }
    Ok(())
}

fn generate_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
