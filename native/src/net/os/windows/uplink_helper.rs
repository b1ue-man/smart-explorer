//! Elevated helper for Internet Connection Sharing on Windows.
//!
//! One-time setup (one UAC prompt) registers a Scheduled Task that runs
//! `se.exe --lan-uplink-helper` with highest privileges on demand and makes
//! the `SharedAccess` service startable. Afterwards the daemon writes a
//! request file, starts the task unelevated, and waits for the response file.
//! The helper re-validates every request against its own view of the
//! network, so a stray request can never enable sharing on a routed link.
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::net::{classify_links, LinkClass, UplinkTarget};

pub(crate) const HELPER_MODE: &str = "--lan-uplink-helper";
const REQUEST_MAX_AGE_SECS: i64 = 60;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HelperOp {
    Enable,
    Disable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HelperRequest {
    pub op: HelperOp,
    pub public_guid: String,
    pub private_guid: String,
    pub issued_at: i64,
    pub nonce: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HelperResponse {
    pub nonce: String,
    pub ok: bool,
    pub message: String,
}

fn directory() -> PathBuf {
    crate::support_dirs::app_data_dir().join("lan_uplink")
}

fn request_path() -> PathBuf {
    directory().join("request.json")
}

fn response_path() -> PathBuf {
    directory().join("response.json")
}

pub(crate) fn task_registered() -> io::Result<bool> {
    super::uplink_install::ready()
}

pub(crate) fn setup_once() -> io::Result<String> {
    super::uplink_install::install()
}

/// Hand one operation to the elevated task and wait for its answer.
pub(crate) fn run_via_task(
    op: HelperOp,
    public: &UplinkTarget,
    private: &UplinkTarget,
) -> io::Result<()> {
    if !crate::net::valid_adapter_id(&public.adapter_id)
        || !crate::net::valid_adapter_id(&private.adapter_id)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Adapter-GUID ist ungueltig",
        ));
    }
    crate::support_dirs::ensure_private_dir(&directory())?;
    if !task_registered()? {
        return Err(io::Error::other(
            "Uplink-Einrichtung fehlt; Reparatur erforderlich",
        ));
    }
    let operation_lock =
        crate::support_dirs::open_private_lock(&directory().join("operation.lock"))?;
    operation_lock.try_lock().map_err(|_| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "Uplink-Vorgang laeuft bereits; erneut versuchen",
        )
    })?;
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let request = HelperRequest {
        op,
        public_guid: public.adapter_id.clone(),
        private_guid: private.adapter_id.clone(),
        issued_at: crate::share::core_now_secs(),
        nonce: nonce.clone(),
    };
    let _ = std::fs::remove_file(response_path());
    crate::support_dirs::write_private_atomic(
        &request_path(),
        &serde_json::to_vec(&request).map_err(io::Error::other)?,
    )?;
    let output = super::uplink_install::powershell(&format!(
        "Start-ScheduledTask -TaskName '{}' -TaskPath '\\'",
        super::uplink_install::task_name()?.replace('\'', "''")
    ))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Aufgabe konnte nicht gestartet werden: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    loop {
        if let Ok(text) = crate::support_dirs::read_private_text(&response_path(), 16 * 1024) {
            if let Ok(response) = serde_json::from_str::<HelperResponse>(&text) {
                if response.nonce == nonce {
                    let _ = std::fs::remove_file(response_path());
                    return if response.ok {
                        Ok(())
                    } else {
                        Err(io::Error::other(response.message))
                    };
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Der elevierte Helfer hat nicht geantwortet (Aufgabe nicht angelegt oder blockiert)",
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Entry point of `se.exe --lan-uplink-helper` (runs elevated).
pub(crate) fn run_helper() -> io::Result<()> {
    super::uplink_install::validate_running_helper()?;
    let text = crate::support_dirs::read_private_text(&request_path(), 16 * 1024)?;
    let request: HelperRequest = serde_json::from_str(&text).map_err(io::Error::other)?;
    let result = validate_and_apply(&request);
    let response = HelperResponse {
        nonce: request.nonce.clone(),
        ok: result.is_ok(),
        message: match &result {
            Ok(message) => message.clone(),
            Err(error) => error.to_string(),
        },
    };
    let _ = std::fs::remove_file(request_path());
    crate::support_dirs::write_private_atomic(
        &response_path(),
        &serde_json::to_vec(&response).map_err(io::Error::other)?,
    )?;
    result.map(|_| ())
}

fn validate_and_apply(request: &HelperRequest) -> io::Result<String> {
    if request.nonce.len() != 32 || !request.nonce.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(io::Error::other("Ungueltige Uplink-Capability"));
    }
    let now = crate::share::core_now_secs();
    if now.saturating_sub(request.issued_at).abs() > REQUEST_MAX_AGE_SECS {
        return Err(io::Error::other("Anforderung ist veraltet"));
    }
    if !crate::net::valid_adapter_id(&request.public_guid)
        || !crate::net::valid_adapter_id(&request.private_guid)
        || request
            .public_guid
            .eq_ignore_ascii_case(&request.private_guid)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Adapter-GUIDs sind ungueltig",
        ));
    }
    let settings = crate::share::LanSettings::load().map_err(io::Error::other)?;
    match request.op {
        HelperOp::Enable => {
            if !settings.uplink_sharing_enabled {
                return Err(io::Error::other(
                    "Internet-Teilen ist in den Einstellungen ausgeschaltet",
                ));
            }
            let facts = crate::net::gather_interface_facts().map_err(io::Error::other)?;
            let links = classify_links(&facts, &[], None, &[]);
            let private_ok = links.iter().any(|(iface, class)| {
                iface.adapter_id.eq_ignore_ascii_case(&request.private_guid)
                    && *class == LinkClass::RouterLess
            });
            if !private_ok {
                return Err(io::Error::other(
                    "Der private Adapter ist kein Link ohne Router; Freigabe verweigert",
                ));
            }
            let private_index = links
                .iter()
                .find(|(iface, _)| iface.adapter_id.eq_ignore_ascii_case(&request.private_guid))
                .map(|(iface, _)| iface.index)
                .ok_or_else(|| io::Error::other("Privater Adapter fehlt"))?;
            crate::share::lan_uplink_evidence::authorize(private_index, &facts)
                .map_err(io::Error::other)?;
            let public_ok = links.iter().any(|(iface, class)| {
                iface.adapter_id.eq_ignore_ascii_case(&request.public_guid)
                    && matches!(class, LinkClass::Uplink | LinkClass::Routed)
            });
            if !public_ok {
                return Err(io::Error::other(
                    "Der oeffentliche Adapter hat keinen Gateway; Freigabe verweigert",
                ));
            }
            super::ics::enable_sharing(&request.public_guid, &request.private_guid)?;
            Ok("Internetverbindungsfreigabe aktiviert".into())
        }
        HelperOp::Disable => {
            super::ics::disable_sharing(&request.public_guid, &request.private_guid)?;
            Ok("Internetverbindungsfreigabe deaktiviert".into())
        }
    }
}
