#[cfg(target_os = "linux")]
use std::time::Duration;

/// An identity-bound handle captured immediately when a legacy helper starts.
pub(crate) struct LegacyParent {
    pid: u32,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(target_os = "linux")]
    identity: Option<LinuxProcessIdentity>,
}

pub(crate) fn bind_legacy_parent(pid: u32) -> Result<LegacyParent, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        let handle = if pid == 0 {
            std::ptr::null_mut()
        } else {
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(87) {
                    return Err(format!(
                        "Legacy-Elternprozess {pid} konnte nicht gebunden werden: {error}"
                    ));
                }
            }
            handle
        };
        Ok(LegacyParent { pid, handle })
    }
    #[cfg(target_os = "linux")]
    {
        let identity = if pid == 0 {
            None
        } else {
            linux_process_identity(pid)?
        };
        Ok(LegacyParent { pid, identity })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = pid;
        Err("PID-Bindung wird auf diesem Betriebssystem nicht unterstuetzt".to_string())
    }
}

impl LegacyParent {
    /// v0.5.119 starts the helper before showing its dialog, so "Later" can
    /// leave this wait pending indefinitely. The captured identity makes that
    /// safe even if the numeric PID is later reused.
    pub(crate) fn wait(self) -> Result<(), String> {
        if self.pid == 0 {
            return Ok(());
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};
            if self.handle.is_null() {
                return Ok(());
            }
            let result = unsafe { WaitForSingleObject(self.handle, INFINITE) };
            if result == WAIT_OBJECT_0 {
                Ok(())
            } else {
                Err(format!(
                    "Warten auf gebundenen Legacy-Elternprozess {} fehlgeschlagen",
                    self.pid
                ))
            }
        }
        #[cfg(target_os = "linux")]
        {
            let Some(identity) = self.identity.as_ref() else {
                return Ok(());
            };
            if identity.is_exited() {
                return Ok(());
            }
            loop {
                match linux_process_identity(self.pid)? {
                    Some(current)
                        if current.start_time == identity.start_time && !current.is_exited() =>
                    {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    _ => return Ok(()),
                }
            }
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        Err("PID-Warten wird auf diesem Betriebssystem nicht unterstuetzt".to_string())
    }
}

#[cfg(windows)]
impl Drop for LegacyParent {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct LinuxProcessIdentity {
    state: char,
    start_time: String,
}

#[cfg(target_os = "linux")]
impl LinuxProcessIdentity {
    fn is_exited(&self) -> bool {
        matches!(self.state, 'Z' | 'X')
    }
}

#[cfg(target_os = "linux")]
fn linux_process_identity(pid: u32) -> Result<Option<LinuxProcessIdentity>, String> {
    let path = format!("/proc/{pid}/stat");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Prozessidentitaet {pid} lesen: {error}")),
    };
    let close = raw
        .rfind(')')
        .ok_or_else(|| format!("Prozessidentitaet {pid} ist ungueltig"))?;
    let fields = raw[close + 1..].split_whitespace().collect::<Vec<_>>();
    let state = fields
        .first()
        .and_then(|value| value.chars().next())
        .ok_or_else(|| format!("Prozessidentitaet {pid} enthaelt keinen Zustand"))?;
    let start_time = fields
        .get(19)
        .ok_or_else(|| format!("Prozessidentitaet {pid} enthaelt keine Startzeit"))?;
    Ok(Some(LinuxProcessIdentity {
        state,
        start_time: (*start_time).to_string(),
    }))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn process_identity_uses_start_time_and_missing_pid_is_absent() {
        assert!(linux_process_identity(std::process::id())
            .unwrap()
            .is_some());
        assert!(linux_process_identity(u32::MAX).unwrap().is_none());
    }

    #[test]
    fn bound_parent_tracks_the_exact_child_until_reaped() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 0.05"])
            .spawn()
            .unwrap();
        let parent = bind_legacy_parent(child.id()).unwrap();
        let reaper = std::thread::spawn(move || child.wait().unwrap());

        parent.wait().unwrap();
        assert!(reaper.join().unwrap().success());
    }

    #[test]
    fn bound_parent_treats_unreaped_zombie_as_exited() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if linux_process_identity(pid)
                .unwrap()
                .is_some_and(|identity| identity.is_exited())
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child never became a zombie"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let parent = bind_legacy_parent(pid).unwrap();

        let started = std::time::Instant::now();
        parent.wait().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(child.wait().unwrap().success());
    }
}
