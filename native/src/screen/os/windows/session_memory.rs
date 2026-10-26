//! All three MEMORYSTATUSEX headroom constraints are required for admission.
use crate::screen::core::frame_memory::MemoryFacts;
pub(super) fn preview_admission() -> crate::screen::core::frame_budget::PreviewAdmission {
    crate::screen::core::frame_budget::PreviewAdmission::FullContract
}
pub(super) fn ceiling(
    facts: Option<MemoryFacts>,
) -> crate::screen::core::frame_memory::CapacityCeiling {
    crate::screen::core::frame_memory::CapacityCeiling::desktop(facts)
}
pub(super) fn observe_ceiling(_ceiling: crate::screen::core::frame_memory::CapacityCeiling) {}
pub(super) fn facts() -> Option<MemoryFacts> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: plain output structure with its mandatory size initialized.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: status remains live and writable during the synchronous call.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    Some(MemoryFacts {
        available_physical: status.ullAvailPhys,
        available_commit: status.ullAvailPageFile,
        available_address_space: status.ullAvailVirtual,
    })
}
