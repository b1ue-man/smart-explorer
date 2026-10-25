//! Windows volume facts use the retained physical ancestry; data uses File handles.
use std::{fs::File, io::{self, Read, Write, Seek, SeekFrom}, sync::atomic::{AtomicBool, Ordering}};
use crate::local_access::ConfinedDirectory;
use super::{CapacitySnapshot, CapacitySource, TargetLimits, VolumeIdentity};
#[path = "confined_local_volume.rs"]
mod volume_info;
#[path = "confined_local_copy.rs"]
mod copy_acceleration;

pub(super) fn qualify(directory: &ConfinedDirectory) -> io::Result<()> {
    // FileFsDeviceInformation reports the actual opened device, even after a
    // drive letter is reassigned. Remote providers retain their own contract.
    let device = volume_info::query(directory.file(), 4, 8)?;
    let characteristics = u32::from_ne_bytes(device[4..8].try_into().map_err(|_| overflow())?);
    if characteristics & 0x10 != 0 { return Err(io::Error::new(io::ErrorKind::Unsupported, "remote Windows volume keeps its provider root contract")); }
    Ok(())
}
pub(super) fn capacity(directory: &ConfinedDirectory, root: &str, identity: String) -> io::Result<CapacitySnapshot> {
    let data = volume_info::query(directory.file(), 7, 32)?; // FileFsFullSizeInformation
    let number = |start: usize| -> io::Result<u64> { u64::try_from(i64::from_ne_bytes(data[start..start+8].try_into().map_err(|_| overflow())?)).map_err(|_| overflow()) };
    let sectors = u32::from_ne_bytes(data[24..28].try_into().map_err(|_| overflow())?);
    let bytes = u32::from_ne_bytes(data[28..32].try_into().map_err(|_| overflow())?);
    let unit = u64::from(sectors).checked_mul(u64::from(bytes)).ok_or_else(overflow)?;
    if unit == 0 { return Err(overflow()); }
    let total = number(0)?.checked_mul(unit).ok_or_else(overflow)?;
    let available = number(8)?.checked_mul(unit).ok_or_else(overflow)?;
    let free = number(16)?.checked_mul(unit).ok_or_else(overflow)?;
    let available = available.min(free);
    let mut result = CapacitySnapshot::known(identity, root, CapacitySource::Filesystem, total, total.saturating_sub(available), available);
    result.read_only = volume_info::facts(directory.file())?.3 & 0x0008_0000 != 0;
    Ok(result)
}
pub(super) fn limits(directory: &ConfinedDirectory) -> io::Result<TargetLimits> {
    let (name, maximum, _, _) = volume_info::facts(directory.file())?;
    Ok(super::fs_profile::windows_profile(&name, maximum).limits)
}
pub(super) fn volume(directory: &ConfinedDirectory) -> io::Result<Option<VolumeIdentity>> {
    let (name, _, serial, _) = volume_info::facts(directory.file())?;
    if serial == 0 { return Ok(None); }
    Ok(Some(VolumeIdentity { volume_id: format!("{serial:016x}"), relative_path: volume_info::inside(directory.file())?, fs_type: name }))
}
fn overflow() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "invalid volume capacity metadata") }
pub(super) fn set_mode(_file: &File, _mode: u32) -> io::Result<()> { Ok(()) }
pub(super) fn mode(_file: &File) -> io::Result<Option<u32>> { Ok(None) }
pub(super) fn sync_filesystem(directory: &ConfinedDirectory) -> io::Result<bool> {
    // Stage data is flushed individually; every namespace-changing NT handle
    // is opened WRITE_THROUGH, so no unprivileged whole-volume flush is needed.
    let _ = volume_info::facts(directory.file())?;
    Ok(true)
}
pub(super) fn confirm_namespace(directory: &ConfinedDirectory) -> io::Result<bool> {
    // Microsoft documents WRITE_THROUGH as flushing metadata changes including
    // renames. create/remove/rename all use that flag on their actual NT handle.
    let _ = volume_info::facts(directory.file())?;
    Ok(true)
}
pub(super) fn copy_permissions(source: &File, stage: &File, _destination: Option<&File>) -> io::Result<()> { stage.set_permissions(source.metadata()?.permissions()) }
pub(super) fn copy(source: &mut File, destination: &mut File, cancel: &AtomicBool, expected: Option<u64>) -> io::Result<u64> {
    if cancel.load(Ordering::Acquire) { return Err(io::Error::new(io::ErrorKind::Interrupted, "local copy cancelled")); }
    let length = expected.unwrap_or(source.metadata()?.len());
    let cloned = copy_acceleration::clone(source, destination, length)?;
    let mut copied = if cloned { length } else { 0 };
    source.seek(SeekFrom::Start(copied))?;
    destination.seek(SeekFrom::Start(copied))?;
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) { return Err(io::Error::new(io::ErrorKind::Interrupted, "local copy cancelled")); }
        let limit = expected.map_or(buffer.len(), |size| size.saturating_sub(copied).min(buffer.len() as u64) as usize);
        if limit == 0 { destination.set_len(copied)?; return Ok(copied); }
        let read = source.read(&mut buffer[..limit])?;
        if read == 0 { destination.set_len(copied)?; return Ok(copied); }
        destination.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
    }
}

pub(super) fn stage_write_access() -> bool { true }
pub(super) fn validate_stage(_file: &File) -> io::Result<()> { Ok(()) }
