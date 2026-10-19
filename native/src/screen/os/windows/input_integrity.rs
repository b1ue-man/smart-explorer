//! The integrity check before every input batch on Windows (SCR-M6, C5):
//! `SendInput` from the host process runs at the user's integrity level and
//! must not drive a window of a higher one (an administrator window). The
//! foreground window's process token is read
//! (`OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken` →
//! `TokenIntegrityLevel`); a higher level than this process's **or a failed
//! query** blocks the batch with `elevated_window` (NR-Nr. 52, A14: the
//! token of an elevated process may be unreadable from medium level, so a
//! failure counts as elevated).
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// The answer of one query: no foreground window (the desktop takes
/// input), this process's own window, or the two integrity levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Foreground {
    None,
    Own,
    Levels { theirs: u32, ours: u32 },
}

/// Whether input must be blocked for this answer; `Err` (the query
/// failed) blocks too.
pub(super) fn blocks(query: Result<Foreground, ()>) -> bool {
    match query {
        Ok(Foreground::None) | Ok(Foreground::Own) => false,
        Ok(Foreground::Levels { theirs, ours }) => theirs > ours,
        Err(()) => true,
    }
}

/// Whether the foreground window belongs to a process of a higher integrity
/// level than this one, or that cannot be told.
pub(super) fn foreground_blocked() -> bool {
    blocks(query())
}

fn query() -> Result<Foreground, ()> {
    // SAFETY: plain queries; the process id is written to a local.
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return Ok(Foreground::None);
    }
    let mut pid = 0u32;
    // SAFETY: `window` is a window handle value, `pid` a local.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid == 0 {
        return Err(());
    }
    // SAFETY: no arguments.
    if pid == unsafe { GetCurrentProcessId() } {
        return Ok(Foreground::Own);
    }
    // SAFETY: a query-only right on a process id; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(());
    }
    let theirs = process_level(process);
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(process) };
    // SAFETY: the pseudo handle of this process needs no closing.
    let ours = process_level(unsafe { GetCurrentProcess() });
    Ok(Foreground::Levels {
        theirs: theirs?,
        ours: ours?,
    })
}

/// The mandatory integrity level (the last sub-authority of the label SID)
/// of `process`.
fn process_level(process: HANDLE) -> Result<u32, ()> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is a local out pointer; closed below.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(());
    }
    let level = token_level(token);
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(token) };
    level
}

fn token_level(token: HANDLE) -> Result<u32, ()> {
    // A label is a SID pointer plus attributes; 256 bytes hold any label.
    let mut buffer = [0u64; 32];
    let mut needed = 0u32;
    // SAFETY: the buffer is 256 bytes, aligned for the structure, and its
    // size is passed; `needed` is a local.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(&buffer) as u32,
            &mut needed,
        )
    };
    if read == 0 {
        return Err(());
    }
    // SAFETY: the call filled a `TOKEN_MANDATORY_LABEL` at the buffer's start.
    let label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let sid = label.Label.Sid;
    if sid.is_null() {
        return Err(());
    }
    // SAFETY: `sid` points into the buffer the call filled; the count is
    // read before the last sub-authority is addressed.
    unsafe {
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return Err(());
        }
        Ok(*GetSidSubAuthority(sid, u32::from(count) - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_task_windows_integrity_query_failure_blocks() {
        // A failed query (the token of an elevated process is unreadable)
        // blocks; so does a higher level; the same or a lower one does not.
        assert!(blocks(Err(())));
        assert!(blocks(Ok(Foreground::Levels {
            theirs: 0x3000,
            ours: 0x2000
        })));
        assert!(!blocks(Ok(Foreground::Levels {
            theirs: 0x2000,
            ours: 0x2000
        })));
        assert!(!blocks(Ok(Foreground::Levels {
            theirs: 0x1000,
            ours: 0x2000
        })));
        assert!(!blocks(Ok(Foreground::None)));
        assert!(!blocks(Ok(Foreground::Own)));
    }
}
