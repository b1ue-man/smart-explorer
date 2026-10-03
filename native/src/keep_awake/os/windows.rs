//! Windows backend of the wake holds (RV1, V4): one power request object per
//! reason with `PowerRequestSystemRequired` (no idle sleep) and
//! `PowerRequestExecutionRequired` (no suspension by process lifetime
//! management), and power throttling (EcoQoS) switched off for the process
//! while any hold lives (B32). `powercfg /requests` shows the reason text.
//! User-initiated sleep is never prevented (Microsoft: requests end when the
//! user puts the machine to sleep).

use std::io;
use std::time::Duration;

use crossbeam_channel::Receiver;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestExecutionRequired,
    PowerRequestSystemRequired, PowerSetRequest, POWER_REQUEST_TYPE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE, REASON_CONTEXT,
    REASON_CONTEXT_0,
};

use super::types::{Applied, Reason};

/// `POWER_REQUEST_CONTEXT_VERSION` (lives in `Win32::System::SystemServices`).
const POWER_REQUEST_CONTEXT_VERSION: u32 = 0;

struct Request {
    handle: HANDLE,
    kinds: Vec<POWER_REQUEST_TYPE>,
    /// The reason text stays alive as long as the request.
    _reason: Vec<u16>,
    note: Option<String>,
}

pub(super) struct Backend {
    requests: [Option<Request>; Reason::ALL.len()],
    throttling_off: bool,
}

impl Backend {
    pub(super) fn new() -> Self {
        Self {
            requests: [None, None, None],
            throttling_off: false,
        }
    }

    pub(super) fn apply(&mut self, held: [bool; Reason::ALL.len()]) -> Applied {
        let mut unavailable = None;
        for reason in Reason::ALL {
            let index = reason.index();
            if held[index] && self.requests[index].is_none() {
                match create_request(reason) {
                    Ok(request) => self.requests[index] = Some(request),
                    Err(error) => unavailable = Some(error),
                }
            } else if !held[index] {
                if let Some(request) = self.requests[index].take() {
                    release(request);
                }
            }
        }
        let any = held.iter().any(|held| *held);
        if any != self.throttling_off {
            match set_throttling_off(any) {
                Ok(()) => self.throttling_off = any,
                Err(error) => {
                    unavailable.get_or_insert(error);
                }
            }
        }
        if unavailable.is_none() {
            unavailable = self
                .requests
                .iter()
                .flatten()
                .find_map(|request| request.note.clone());
        }
        Applied {
            engaged: self.requests.iter().any(Option::is_some),
            throttling_off: self.throttling_off,
            unavailable,
        }
    }

    pub(super) fn renew_after(&self) -> Option<Duration> {
        None
    }

    pub(super) fn wake_receiver(&self) -> Option<Receiver<()>> {
        None
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        for slot in &mut self.requests {
            if let Some(request) = slot.take() {
                release(request);
            }
        }
        if self.throttling_off {
            let _ = set_throttling_off(false);
        }
    }
}

fn create_request(reason: Reason) -> Result<Request, String> {
    let mut text: Vec<u16> = format!("Smart Explorer: {}", reason.label())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let context = REASON_CONTEXT {
        Version: POWER_REQUEST_CONTEXT_VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 {
            SimpleReasonString: text.as_mut_ptr(),
        },
    };
    // SAFETY: the context and its NUL-terminated text outlive the call.
    let handle = unsafe { PowerCreateRequest(&context) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Windows-Energieanforderung nicht möglich: {}",
            io::Error::last_os_error()
        ));
    }
    let mut kinds = Vec::new();
    let mut missing = Vec::new();
    for (kind, label) in [
        (PowerRequestSystemRequired, "SystemRequired"),
        (PowerRequestExecutionRequired, "ExecutionRequired"),
    ] {
        // SAFETY: `handle` is a live power request object.
        if unsafe { PowerSetRequest(handle, kind) } != 0 {
            kinds.push(kind);
        } else {
            missing.push(format!("{label}: {}", io::Error::last_os_error()));
        }
    }
    if kinds.is_empty() {
        let error = io::Error::last_os_error();
        // SAFETY: closes the object created above exactly once.
        unsafe { CloseHandle(handle) };
        return Err(format!("Windows-Energieanforderung abgelehnt: {error}"));
    }
    Ok(Request {
        handle,
        kinds,
        _reason: text,
        note: (!missing.is_empty()).then(|| {
            format!(
                "Wachhalten nur teilweise verfügbar ({})",
                missing.join("; ")
            )
        }),
    })
}

fn release(request: Request) {
    for kind in &request.kinds {
        // SAFETY: clears a request type set on this live object.
        unsafe { PowerClearRequest(request.handle, *kind) };
    }
    // SAFETY: closes the object exactly once.
    unsafe { CloseHandle(request.handle) };
}

/// `true` opts the process out of execution-speed throttling (EcoQoS);
/// `false` lets the system decide again.
fn set_throttling_off(off: bool) -> Result<(), String> {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: if off {
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED
        } else {
            0
        },
        StateMask: 0,
    };
    // SAFETY: the pseudo handle of the own process and a correctly sized
    // structure that outlives the call.
    let done = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    if done == 0 {
        Err(format!(
            "Windows-Stromdrosselung nicht abschaltbar: {}",
            io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}
