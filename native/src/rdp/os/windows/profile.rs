//! Owner-protected native client profile. Kept read-only and non-replaceable
//! until the exact spawned client exits; contains no username or password.
use std::{fs::{File, OpenOptions}, io::{Read, Write}, net::SocketAddr, path::PathBuf};
use std::os::windows::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}, io::FromRawHandle};
use windows_sys::Win32::{Foundation::{INVALID_HANDLE_VALUE, LocalFree},
    Security::{Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES},
    Storage::FileSystem::{CreateDirectoryW, CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE}};

pub(super) struct Profile { pub(super) path:PathBuf, directory:PathBuf, file:Option<File>, parent:Option<File> }
impl Profile {
    pub(super) fn create(listener:SocketAddr, alias:Option<&str>) -> Result<Self,String> {
        let token = crate::share::tcp_forward::TunnelId::generate()?;
        let directory = std::env::temp_dir().join(format!("smart-explorer-rdp-{}",token.as_str()));
        let path = directory.join("session.rdp");
        let descriptor = Descriptor::new()?;
        let security = SECURITY_ATTRIBUTES { nLength:std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor:descriptor.0,bInheritHandle:0 };
        let directory_w:Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: terminated path and self-relative descriptor live through the call.
        if unsafe { CreateDirectoryW(directory_w.as_ptr(),&security) } == 0 {
            return Err(format!("Cannot create private RDP profile directory: {}",std::io::Error::last_os_error()));
        }
        let mut profile = Self {path,directory,file:None,parent:None};
        let parent = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ|FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&profile.directory).map_err(|e|format!("Cannot retain private RDP directory: {e}"))?;
        let metadata = parent.metadata().map_err(|e|e.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("RDP profile directory is not a plain directory".into());
        }
        profile.parent = Some(parent);
        let mut body = format!("full address:s:{listener}\r\nauthentication level:i:2\r\nenablecredsspsupport:i:1\r\n");
        if let Some(alias) = alias {
            if !crate::rdp::core::PublishedApp::valid_alias(alias) { return Err("Invalid published RemoteApp alias".into()); }
            body.push_str(&format!("disableconnectionsharing:i:1\r\nremoteapplicationmode:i:1\r\nremoteapplicationprogram:s:||{alias}\r\nremoteapplicationname:s:{alias}\r\nalternate shell:s:||{alias}\r\n"));
        } else {
            body.push_str("remoteapplicationmode:i:0\r\n");
        }
        let mut expected = vec![0xff,0xfe];
        expected.extend(body.encode_utf16().flat_map(u16::to_le_bytes));
        let path_w:Vec<u16> = profile.path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: a new file is created atomically with an explicit owner-only
        // protected DACL. The returned handle is uniquely transferred to File.
        let handle = unsafe { CreateFileW(path_w.as_ptr(),0x40000000,FILE_SHARE_READ,&security,
            CREATE_NEW,FILE_FLAG_OPEN_REPARSE_POINT,std::ptr::null_mut()) };
        if handle == INVALID_HANDLE_VALUE { return Err(format!("Cannot create private RDP profile: {}",std::io::Error::last_os_error())); }
        // SAFETY: handle is a successful CreateFileW result owned exactly once.
        let mut writer = unsafe { File::from_raw_handle(handle) };
        writer.write_all(&expected).map_err(|e|format!("Cannot write RDP profile: {e}"))?;
        writer.sync_all().map_err(|e|format!("Cannot flush RDP profile: {e}"))?;
        drop(writer);
        let mut file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&profile.path)
            .map_err(|e|format!("Cannot pin RDP profile read-only: {e}"))?;
        let metadata = file.metadata().map_err(|e|e.to_string())?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || metadata.len() != expected.len() as u64 { return Err("RDP profile changed before native client start".into()); }
        let mut actual = Vec::new();
        Read::by_ref(&mut file).take(expected.len() as u64+1).read_to_end(&mut actual).map_err(|e|e.to_string())?;
        if actual != expected { return Err("RDP profile contents changed before native client start".into()); }
        profile.file = Some(file);
        Ok(profile)
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        self.file.take(); self.parent.take();
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
struct Descriptor(*mut std::ffi::c_void);
impl Descriptor {
    fn new()->Result<Self,String> {
        let sddl:Vec<u16> = "D:P(A;;FA;;;OW)".encode_utf16().chain(Some(0)).collect();
        let mut pointer = std::ptr::null_mut();
        // SAFETY: fixed SDDL, current revision, valid output pointer.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut pointer,std::ptr::null_mut()) } == 0 {
            return Err(format!("Cannot create owner-only RDP profile ACL: {}",std::io::Error::last_os_error()));
        }
        Ok(Self(pointer))
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: the conversion API allocates this descriptor with LocalAlloc.
        unsafe { LocalFree(self.0); }
    }
}
