//! Mounted removable volumes. Open mountinfo before the first snapshot;
//! POLLPRI reports mount namespace changes. udev/sysfs classify USB HDDs too.
use super::DriveInfo;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
static STATE: OnceLock<Arc<Mutex<Option<Vec<DriveInfo>>>>> = OnceLock::new();
pub(super) fn snapshot() -> Option<Vec<DriveInfo>> {
    let state = STATE.get_or_init(|| {
        let state = Arc::new(Mutex::new(None));
        let worker = state.clone();
        let _ = std::thread::Builder::new()
            .name("sync-mount-events".into())
            .spawn(move || monitor(worker));
        state
    });
    state.lock().unwrap_or_else(PoisonError::into_inner).clone()
}
fn monitor(state: Arc<Mutex<Option<Vec<DriveInfo>>>>) {
    loop {
        let mut file = match File::open("/proc/self/mountinfo") {
            Ok(file) => file,
            Err(error) => {
                crate::daemon::log_worker(&format!("volume monitor: {error}"));
                std::thread::sleep(std::time::Duration::from_secs(60));
                continue;
            }
        };
        loop {
            match read(&mut file) {
                Ok(drives) => *state.lock().unwrap_or_else(PoisonError::into_inner) = Some(drives),
                Err(error) => {
                    crate::daemon::log_worker(&format!("volume snapshot: {error}"));
                    break;
                }
            }
            let mut descriptor = libc::pollfd {
                fd: file.as_raw_fd(),
                events: libc::POLLPRI,
                revents: 0,
            };
            // Re-read after an event or the readiness fallback (udev properties
            // can appear slightly after the mount event).
            let ready = unsafe { libc::poll(&mut descriptor, 1, 5000) };
            if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}
fn read(file: &mut File) -> io::Result<Vec<DriveInfo>> {
    file.seek(SeekFrom::Start(0))?;
    let mut body = String::new();
    Read::by_ref(file)
        .take(4 * 1024 * 1024 + 1)
        .read_to_string(&mut body)?;
    if body.len() > 4 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mount table exceeds safety bound",
        ));
    }
    let mut drives = Vec::new();
    for line in body.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let Some(separator) = fields.iter().position(|field| *field == "-") else {
            continue;
        };
        if fields.len() < separator + 4 || separator < 6 {
            continue;
        }
        let device = fields[2];
        if !device.split_once(':').is_some_and(|(major, minor)| {
            major.bytes().all(|byte| byte.is_ascii_digit())
                && minor.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            continue;
        }
        let sys = std::fs::canonicalize(format!("/sys/dev/block/{device}")).ok();
        let properties =
            std::fs::read_to_string(format!("/run/udev/data/b{device}")).unwrap_or_default();
        let property = |key: &str| {
            properties
                .lines()
                .find_map(|line| line.strip_prefix(&format!("E:{key}=")))
                .map(str::to_string)
        };
        let external = property("ID_BUS")
            .is_some_and(|bus| matches!(bus.as_str(), "usb" | "ieee1394"))
            || sys.as_ref().is_some_and(|path| {
                path.components().any(|part| {
                    part.as_os_str().to_str().is_some_and(|part| {
                        part.strip_prefix("usb").is_some_and(|suffix| {
                            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                        })
                    })
                })
            })
            || sys.as_ref().is_some_and(|path| {
                path.ancestors().take(4).any(|ancestor| {
                    std::fs::read_to_string(ancestor.join("type"))
                        .is_ok_and(|kind| kind.trim() == "SD")
                })
            })
            || sys.as_ref().is_some_and(|path| {
                path.ancestors().take(3).any(|ancestor| {
                    std::fs::read_to_string(ancestor.join("removable"))
                        .is_ok_and(|value| value.trim() == "1")
                })
            });
        if !external {
            continue;
        }
        let serial = property("ID_FS_UUID")
            .or_else(|| property("ID_PART_ENTRY_UUID"))
            .unwrap_or_else(|| {
                format!(
                    "{device}:{}:{}",
                    unescape(fields[separator + 2]),
                    sys.as_ref()
                        .map_or(String::new(), |path| path.display().to_string())
                )
            });
        drives.push(DriveInfo {
            letter: unescape(fields[4]),
            label: property("ID_FS_LABEL").unwrap_or_default(),
            serial,
        });
    }
    Ok(drives)
}
fn unescape(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\'
            && at + 3 < bytes.len()
            && bytes[at + 1..at + 4]
                .iter()
                .all(|byte| matches!(byte, b'0'..=b'7'))
        {
            out.push(
                ((bytes[at + 1] - b'0') as u16 * 64
                    + (bytes[at + 2] - b'0') as u16 * 8
                    + (bytes[at + 3] - b'0') as u16) as u8,
            );
            at += 4;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
