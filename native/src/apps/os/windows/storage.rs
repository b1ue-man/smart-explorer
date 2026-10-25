use std::{fs,io,path::Path,ptr};
use std::os::windows::{ffi::OsStrExt,fs::MetadataExt};
use windows_sys::Win32::{Foundation::*,Security::{*,Authorization::*},Storage::FileSystem::*};

pub(crate) fn prepare_journal(root:&Path)->io::Result<()> {
    fs::create_dir_all(root)?;
    let meta=fs::symlink_metadata(root)?;
    if !meta.is_dir() || meta.file_attributes()&FILE_ATTRIBUTE_REPARSE_POINT!=0 {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"launch ledger cannot be a link/reparse point"));}
    let sid=super::session::sid()?;
    let path:Vec<u16>=root.as_os_str().encode_wide().chain(Some(0)).collect();
    let sid_text:Vec<u16>=sid.encode_utf16().chain(Some(0)).collect();
    let mut expected_sid=ptr::null_mut();
    if unsafe {ConvertStringSidToSidW(sid_text.as_ptr(),&mut expected_sid)}==0 {return Err(io::Error::last_os_error());}
    let mut owner=ptr::null_mut();let mut existing=ptr::null_mut();
    let status=unsafe {GetNamedSecurityInfoW(path.as_ptr(),SE_FILE_OBJECT,OWNER_SECURITY_INFORMATION,&mut owner,
        ptr::null_mut(),ptr::null_mut(),ptr::null_mut(),&mut existing)};
    let same=status==ERROR_SUCCESS && !owner.is_null() && unsafe {EqualSid(owner,expected_sid)}!=0;
    unsafe {LocalFree(expected_sid);if !existing.is_null() {LocalFree(existing);}}
    if !same {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"launch ledger owner is not the interactive user"));}
    let sddl:Vec<u16>=format!("D:P(A;OICI;FA;;;{sid})").encode_utf16().chain(Some(0)).collect();
    let mut descriptor=ptr::null_mut();
    if unsafe {ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),SDDL_REVISION_1,&mut descriptor,ptr::null_mut())}==0 {return Err(io::Error::last_os_error());}
    let result=unsafe {SetFileSecurityW(path.as_ptr(),DACL_SECURITY_INFORMATION|PROTECTED_DACL_SECURITY_INFORMATION,descriptor)};
    unsafe {LocalFree(descriptor);}
    if result==0 {return Err(io::Error::last_os_error());}
    Ok(())
}
pub(crate) fn sync_journal(_root:&Path)->io::Result<()> {
    // `publish_record` uses MOVEFILE_WRITE_THROUGH; individual files are flushed
    // before their rename. Windows does not support ordinary directory fsync.
    Ok(())
}
pub(crate) fn publish_record(file:tempfile::NamedTempFile,path:&Path,replace:bool)->io::Result<bool> {
    let source:Vec<u16>=file.path().as_os_str().encode_wide().chain(Some(0)).collect();
    let target:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags=MOVEFILE_WRITE_THROUGH|if replace {MOVEFILE_REPLACE_EXISTING} else {0};
    if unsafe {MoveFileExW(source.as_ptr(),target.as_ptr(),flags)}==0 {
        let error=io::Error::last_os_error();
        if !replace && matches!(error.raw_os_error(),Some(e) if e==ERROR_ALREADY_EXISTS as i32||e==ERROR_FILE_EXISTS as i32) {return Ok(false);}
        return Err(error);
    }
    Ok(true)
}
