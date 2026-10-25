//! Full Windows file identity for durable file publication receipts.
use std::{io, path::Path};
#[path = "windows_file_proof.rs"]
mod windows_file_proof;
pub(crate) use windows_file_proof::{same_file_object, stable_handle_identity};

pub(crate) fn stable_path_identity(path: &Path) -> io::Result<String> {
    let name = path.file_name().ok_or_else(|| io::Error::new(
        io::ErrorKind::InvalidInput, "file identity requires a child name"))?;
    let parent_path = path.parent().filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = crate::local_access::ConfinedDirectory::open_root(parent_path)?;
    let selected = parent.entry(name)?;
    let identity = crate::copy::local_file_identity_handle(&selected)?;
    if crate::copy::local_file_identity_handle(&parent.entry(name)?)? != identity {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file changed during identity query"));
    }
    Ok(identity)
}
