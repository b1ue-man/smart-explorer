//! Tokens retain actual product allocations; closing a channel cannot free them.
use std::{collections::BTreeMap, io, sync::{Mutex, PoisonError}};
use crate::screen::core::{capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice},
    capture_bytes::AdmittedBytes};
enum Resource {External(CaptureSlice), Buffer(AdmittedBytes)}
#[derive(Default)]
struct Arena {next: u64, entries: BTreeMap<u64, Resource>}
static ARENA: Mutex<Arena> = Mutex::new(Arena {next: 0, entries: BTreeMap::new()});
fn owner(value: u32) -> io::Result<CaptureOwner> {
    use CaptureOwner::*;
    match value {0 => Ok(NativeSurface), 1 => Ok(Readback), 2 => Ok(SourcePixels),
        6 => Ok(Metadata), 7 => Ok(Transport), 8 => Ok(CodecInput), 9 => Ok(CodecOutput),
        _ => Err(io::Error::other("invalid Android capture owner"))}
}
pub(crate) fn claim(grant: &CaptureGrant, owner_id: u32, bytes: u64, buffer: bool) -> io::Result<u64> {
    if bytes == 0 || bytes > i32::MAX as u64 {return Err(io::Error::other("invalid Android capture allocation size"));}
    let owner = owner(owner_id)?;
    let resource = if buffer {Resource::Buffer(AdmittedBytes::new(grant, owner, bytes)?)}
        else {Resource::External(grant.claim(owner, bytes)?)};
    let mut arena = ARENA.lock().map_err(|_| io::Error::other("Android capture arena poisoned"))?;
    arena.next = arena.next.checked_add(1).filter(|id| *id <= i64::MAX as u64)
        .ok_or_else(|| io::Error::other("Android capture tokens exhausted"))?;
    let id = arena.next;
    arena.entries.insert(id, resource);
    Ok(id)
}
/// The address remains stable until release(token). Kotlin's owner synchronizes
/// every access and releases only after its final reader/encoder/callback exits.
pub(crate) fn address(token: u64) -> Option<(usize, usize)> {
    let mut arena = ARENA.lock().ok()?;
    match arena.entries.get_mut(&token)? {
        Resource::Buffer(bytes) => Some((bytes.as_mut_slice().as_mut_ptr() as usize, bytes.len())),
        Resource::External(slice) => {let _ = slice.bytes(); None}
    }
}
pub(crate) fn release(token: u64) {
    let removed = ARENA.lock().unwrap_or_else(PoisonError::into_inner).entries.remove(&token);
    drop(removed);
}
#[cfg(test)]
#[path="batches_capture_task_tests.rs"]
mod tests;
