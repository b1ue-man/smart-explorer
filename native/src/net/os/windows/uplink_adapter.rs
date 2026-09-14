//! `UplinkAdapter` for Windows: ICS through the elevated scheduled-task
//! helper, internet verdict through WinRT `NetworkInformation`.
use std::time::{Duration, Instant};

use crate::net::{Facility, InterfaceFacts, UplinkAdapter, UplinkTarget};

use super::{ics, uplink_helper};

const PROBE_CACHE: Duration = Duration::from_secs(60);

pub(crate) struct WindowsIcsAdapter {
    probed_at: Option<Instant>,
    probed_setup_done: bool,
    facility: Facility,
}

impl WindowsIcsAdapter {
    pub(crate) fn new() -> Self {
        Self {
            probed_at: None,
            probed_setup_done: false,
            facility: Facility::Unavailable("noch nicht geprueft".into()),
        }
    }

    fn probe_now(setup_done: bool) -> Facility {
        match ics::sharing_installed() {
            Ok(true) => {}
            Ok(false) => {
                return Facility::Unavailable(
                    "Internetverbindungsfreigabe (ICS) ist auf dieser Windows-Edition nicht installiert"
                        .into(),
                )
            }
            Err(error) => {
                return Facility::Unavailable(format!("ICS nicht abfragbar: {error}"));
            }
        }
        match ics::shared_access_startup() {
            Ok(mode) if mode.eq_ignore_ascii_case("Disabled") && setup_done => {
                return Facility::Unavailable(
                    "Dienst 'SharedAccess' ist deaktiviert; Einrichtung erneut ausfuehren".into(),
                );
            }
            Ok(_) => {}
            Err(error) => {
                return Facility::Unavailable(format!("ICS-Dienst nicht abfragbar: {error}"));
            }
        }
        if !setup_done {
            return Facility::Unavailable(
                "einmalige Einrichtung (Windows-UAC) noch nicht durchgefuehrt".into(),
            );
        }
        match uplink_helper::task_registered() {
            Ok(true) => Facility::Available,
            Ok(false) => Facility::Unavailable(
                "Aufgabe 'Smart Explorer LAN-Uplink' fehlt; Einrichtung erneut ausfuehren".into(),
            ),
            Err(error) => {
                Facility::Unavailable(format!("Aufgabenplanung nicht abfragbar: {error}"))
            }
        }
    }
}

impl UplinkAdapter for WindowsIcsAdapter {
    fn probe(&mut self, setup_done: bool) -> Facility {
        let fresh = self
            .probed_at
            .is_some_and(|at| at.elapsed() < PROBE_CACHE && self.probed_setup_done == setup_done);
        if !fresh {
            self.facility = Self::probe_now(setup_done);
            self.probed_at = Some(Instant::now());
            self.probed_setup_done = setup_done;
        }
        self.facility.clone()
    }

    fn setup_once(&mut self) -> Result<String, String> {
        let message = uplink_helper::setup_once().map_err(|error| error.to_string())?;
        self.probed_at = None;
        Ok(message)
    }

    fn enable(&mut self, private: &UplinkTarget, public: &UplinkTarget) -> Result<(), String> {
        uplink_helper::run_via_task(uplink_helper::HelperOp::Enable, public, private)
            .map_err(|error| error.to_string())
    }

    fn disable(&mut self, private: &UplinkTarget, public: &UplinkTarget) -> Result<(), String> {
        uplink_helper::run_via_task(uplink_helper::HelperOp::Disable, public, private)
            .map_err(|error| error.to_string())
    }

    fn sharing_active(&mut self, private: &UplinkTarget) -> Result<Option<bool>, String> {
        ics::private_sharing_active(&private.adapter_id)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn internet_ifaces(&mut self, facts: &[InterfaceFacts]) -> Option<Vec<u32>> {
        use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation};
        let adapter_guid = (|| -> windows::core::Result<Option<String>> {
            let profile = NetworkInformation::GetInternetConnectionProfile()?;
            if profile.GetNetworkConnectivityLevel()? != NetworkConnectivityLevel::InternetAccess {
                return Ok(None);
            }
            let adapter = profile.NetworkAdapter()?;
            let id = adapter.NetworkAdapterId()?;
            Ok(Some(format!("{{{id:?}}}")))
        })()
        .ok()?;
        let adapter_guid = adapter_guid?;
        Some(
            facts
                .iter()
                .filter(|iface| iface.adapter_id.eq_ignore_ascii_case(&adapter_guid))
                .map(|iface| iface.index)
                .collect(),
        )
    }
}

pub(crate) fn run_helper_if_requested(
    arguments: &[std::ffi::OsString],
) -> Option<std::io::Result<()>> {
    if arguments.len() == 1 && arguments[0] == std::ffi::OsStr::new(uplink_helper::HELPER_MODE) {
        Some(uplink_helper::run_helper())
    } else {
        None
    }
}
