//! Rooted no-reparse pins, trusted owners/ACLs, denied write/delete on image.
use std::{ffi::{c_void, OsString}, fs::{File, OpenOptions}, io, os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, io::AsRawHandle}, path::{Component, Path, PathBuf}, ptr::null_mut};
use windows_sys::Win32::{Foundation::{LocalFree, GENERIC_ALL, GENERIC_READ, GENERIC_WRITE}, Security::{self, Authorization::{GetSecurityInfo, SE_FILE_OBJECT}, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION}, Storage::FileSystem::{DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL, WRITE_DAC, WRITE_OWNER}, UI::Shell::{SHGetKnownFolderPath, FOLDERID_ProgramData, FOLDERID_ProgramFiles}, System::Com::CoTaskMemFree};
use crate::login::os::shared::{denied, installation::Paths};
use super::security::sid_string;
pub(crate) struct ProtectedFile { pub file: File, _parents: Vec<File> }
fn known(id: &windows_sys::core::GUID) -> io::Result<PathBuf> {
    let mut text = null_mut();
    let code = unsafe { SHGetKnownFolderPath(id, 0, null_mut(), &mut text) };
    if code < 0 || text.is_null() { return Err(io::Error::other("Windows machine installation directory is unavailable")); }
    let mut len = 0; unsafe { while *text.add(len) != 0 { len += 1; } }
    let path = PathBuf::from(OsString::from_wide(unsafe { std::slice::from_raw_parts(text, len) }));
    unsafe { CoTaskMemFree(text.cast()); } Ok(path)
}
pub(crate) fn paths() -> io::Result<Paths> { Ok(Paths { config: known(&FOLDERID_ProgramData)?.join("Smart Explorer Login"), releases: known(&FOLDERID_ProgramFiles)?.join("Smart Explorer Login").join("releases"), image_name: "smart-explorer-login.exe" }) }
fn trusted(sid: &str) -> bool { matches!(sid, "S-1-5-18" | "S-1-5-32-544" | "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464") }
pub(crate) fn open_protected(path: &Path, executable: bool) -> io::Result<ProtectedFile> {
    if !path.is_absolute() { return Err(denied("login trust path must be absolute")); }
    let mut parents = Vec::new(); let mut current = PathBuf::new();
    let parts: Vec<_> = path.components().collect();
    for (index, component) in parts.iter().enumerate() {
        match component { Component::Prefix(_) | Component::RootDir | Component::Normal(_) => current.push(component.as_os_str()), _ => return Err(denied("login trust path contains traversal")) }
        if matches!(component, Component::Prefix(_)) { continue; }
        let directory = index + 1 != parts.len();
        let file = OpenOptions::new().access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES | if directory { 0 } else { GENERIC_READ })
            .share_mode(FILE_SHARE_READ | if directory { FILE_SHARE_WRITE } else { 0 })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(&current)?;
        validate(&file, directory, executable && !directory)?;
        if directory { parents.push(file); } else { return Ok(ProtectedFile { file, _parents: parents }); }
    }
    Err(denied("login trust path has no file"))
}
fn validate(file: &File, directory: bool, _executable: bool) -> io::Result<()> { validate_for(file, directory, None, false) }
pub(super) fn validate_public_status(file: &File) -> io::Result<()> { validate(file, false, false) }
fn validate_for(file: &File, directory: bool, gateway: Option<&str>, private: bool) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || metadata.is_dir() != directory || (!directory && !metadata.is_file()) { return Err(denied("login trust path is a reparse point or wrong file kind")); }
    let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 { return Err(io::Error::last_os_error()); }
    if !directory && info.nNumberOfLinks != 1 { return Err(denied("login trust file has multiple hard links")); }
    let mut owner = null_mut(); let mut acl: *mut ACL = null_mut(); let mut descriptor = null_mut();
    let code = unsafe { GetSecurityInfo(file.as_raw_handle().cast(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, &mut owner, null_mut(), &mut acl, null_mut(), &mut descriptor) };
    if code != 0 { return Err(io::Error::from_raw_os_error(code as i32)); }
    let result = (|| {
        if owner.is_null() || (!trusted(&sid_string(owner)?) && gateway != Some(sid_string(owner)?.as_str())) || acl.is_null() { return Err(denied("login trust file requires an administrative owner and explicit DACL")); }
        let dangerous = GENERIC_ALL | GENERIC_WRITE | DELETE | WRITE_DAC | WRITE_OWNER | 0x40 | 0x100 | 0x10 | if directory { 0 } else { 0x2 | 0x4 };
        for index in 0..unsafe { (*acl).AceCount } {
            let mut raw: *mut c_void = null_mut();
            if unsafe { Security::GetAce(acl, index as u32, &mut raw) } == 0 { return Err(io::Error::last_os_error()); }
            let header = unsafe { &*raw.cast::<ACE_HEADER>() };
            if header.AceFlags & 0x08 != 0 { continue; } // INHERIT_ONLY_ACE does not apply to this object.
            if header.AceType == 0 { // ACCESS_ALLOWED_ACE_TYPE
                if usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>() { return Err(denied("invalid login trust ACE")); }
                let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
                let sid = sid_string((&ace.SidStart as *const u32).cast_mut().cast())?;
                if private && !trusted(&sid) && gateway != Some(sid.as_str()) && sid != "S-1-3-4" { return Err(denied("private gateway record grants access outside its exact service identity")); }
                if ace.Mask & dangerous != 0 && !trusted(&sid) && gateway != Some(sid.as_str()) && sid != "S-1-3-4" { return Err(denied("login trust path grants mutation to an unprivileged principal")); }
            } else if header.AceType != 1 && header.AceType != 2 && header.AceType != 17 { return Err(denied("login trust DACL uses unsupported conditional/object ACEs")); }
        }
        Ok(())
    })();
    unsafe { LocalFree(descriptor); } result
}

pub(crate) fn gateway_state_directory() -> io::Result<PathBuf> { Ok(known(&FOLDERID_ProgramData)?.join("Smart Explorer Login Gateway")) }

pub(super) fn open_gateway_record(name: &str, max: u64) -> io::Result<ProtectedFile> {
    let policy = crate::login::os::shared::installation::policy()?;
    super::pipe::require_gateway(&policy)?;
    let root = gateway_state_directory()?;
    let path = root.join(name); let mut current = PathBuf::new(); let mut parents = Vec::new();
    let parts: Vec<_> = path.components().collect();
    for (index, component) in parts.iter().enumerate() {
        match component { Component::Prefix(_) | Component::RootDir | Component::Normal(_) => current.push(component.as_os_str()), _ => return Err(denied("gateway state path contains traversal")) }
        if matches!(component, Component::Prefix(_)) { continue; }
        let directory = index + 1 != parts.len();
        let file = OpenOptions::new().access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES | if directory { 0 } else { GENERIC_READ })
            .share_mode(FILE_SHARE_READ | if directory { FILE_SHARE_WRITE } else { 0 })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(&current)?;
        if current.starts_with(&root) { validate_for(&file, directory, Some(&policy.gateway_identity), true)?; }
        else { validate(&file, directory, false)?; }
        if directory { parents.push(file); }
        else { if file.metadata()?.len() > max { return Err(denied("private gateway record exceeds limit")); } return Ok(ProtectedFile { file, _parents: parents }); }
    }
    Err(denied("gateway record is missing"))
}
pub(crate) fn read_gateway_key() -> io::Result<Vec<u8>> { read_gateway_record(crate::login::core::gateway_types::MACHINE_FILE, 8192) }
fn read_gateway_record(name: &str, max: usize) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let pinned = open_gateway_record(name, max as u64)?;
    let mut bytes = Vec::new(); pinned.file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max { return Err(denied("private gateway record exceeds limit")); } Ok(bytes)
}
pub(crate) fn read_gateway_network() -> io::Result<Option<Vec<u8>>> {
    match read_gateway_record(crate::login::core::gateway_types::NETWORK_FILE, crate::login::core::gateway_types::MAX_NETWORK) { Ok(bytes) => Ok(Some(bytes)), Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None), Err(error) => Err(error) }
}
pub(crate) fn write_gateway_address(bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if bytes.len() > 32768 { return Err(denied("public machine address exceeds limit")); }
    let _pins = open_gateway_record(crate::login::core::gateway_types::MACHINE_FILE, 8192)?;
    let root = gateway_state_directory()?;
    let temporary = root.join(format!(".address-{}", crate::login::os::shared::nonce()?));
    let mut file = OpenOptions::new().write(true).create_new(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&temporary)?;
    let result = (|| { file.write_all(bytes)?; file.sync_all()?; drop(file); std::fs::rename(&temporary, root.join(crate::login::core::gateway_types::ADDRESS_FILE)) })();
    if result.is_err() { let _ = std::fs::remove_file(temporary); } result
}
/// Fresh administrative directory only; never modifies an active policy/service.
pub(crate) fn write_provision_bundle(directory: &Path, records: &[(&str, &[u8])]) -> io::Result<()> {
    use std::io::Write;
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
    super::security::require_administrator()?;
    if !directory.is_absolute() || directory.file_name().is_none() { return Err(denied("provisioning output must be a new absolute directory")); }
    fn normalized(path: &Path) -> io::Result<Vec<String>> {
        use std::path::Prefix;
        path.components().map(|component| match component {
            Component::Prefix(prefix) => match prefix.kind() { Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => Ok(format!("{}:", (drive as char).to_ascii_lowercase())), _ => Err(denied("provisioning output must use a local Windows volume")) },
            Component::RootDir => Ok("\\".into()), Component::Normal(name) => Ok(name.to_string_lossy().to_lowercase()),
            _ => Err(denied("provisioning output contains traversal")),
        }).collect()
    }
    let output = normalized(directory)?;
    let active = paths()?; let state = gateway_state_directory()?;
    for root in [&active.config, &active.releases, &state] {
        let root = normalized(root)?;
        if output.starts_with(&root) || root.starts_with(&output) { return Err(denied("offline provisioning must remain outside active login installation/state")); }
    }
    let parent = directory.parent().ok_or_else(|| denied("provisioning directory has no parent"))?;
    let mut ancestry = Vec::new(); let mut current = PathBuf::new();
    for component in parent.components() {
        match component { Component::Prefix(_) | Component::RootDir | Component::Normal(_) => current.push(component.as_os_str()), _ => return Err(denied("provisioning path contains traversal")) }
        if matches!(component, Component::Prefix(_)) { continue; }
        let file = OpenOptions::new().access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(&current)?;
        validate(&file, true, false)?; ancestry.push(file);
    }
    let descriptor = super::security::Descriptor::new("O:BAG:BAD:P(A;OICI;GA;;;SY)(A;OICI;GA;;;BA)")?;
    let path = super::security::wide(&directory.to_string_lossy());
    if unsafe { CreateDirectoryW(path.as_ptr(), &descriptor.attributes()) } == 0 { return Err(io::Error::last_os_error()); }
    let mut created = Vec::new();
    let result = (|| {
        for (name, bytes) in records {
            if name.contains(['/', '\\', ':', '\0']) || name.is_empty() || bytes.len() > 1048576 { return Err(denied("invalid private provisioning record")); }
            let path = directory.join(name);
            use std::os::windows::io::FromRawHandle;
            let file_descriptor = super::security::Descriptor::new("O:BAG:BAD:P(A;;GA;;;SY)(A;;GA;;;BA)")?;
            let raw = unsafe { windows_sys::Win32::Storage::FileSystem::CreateFileW(super::security::wide(&path.to_string_lossy()).as_ptr(), GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ, &file_descriptor.attributes(), windows_sys::Win32::Storage::FileSystem::CREATE_NEW, FILE_FLAG_OPEN_REPARSE_POINT, null_mut()) };
            let owned = super::security::Handle::new(raw)?;
            let mut file = unsafe { File::from_raw_handle(owned.0.cast()) }; std::mem::forget(owned);
            created.push(path); file.write_all(bytes)?; file.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() { for file in created { let _ = std::fs::remove_file(file); } let _ = std::fs::remove_dir(directory); }
    result
}
