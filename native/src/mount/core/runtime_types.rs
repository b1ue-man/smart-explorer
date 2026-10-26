//! Pure filesystem-runtime version domains, reports and installer outcomes.
pub(crate) const DOKANY_LIBRARY_API_VERSION: u32 = 231;
pub(crate) const DOKANY_DRIVER_PROTOCOL_VERSION: u32 = 0x0000_0190;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DokanyVersionCompatibilityError {
    LibraryApiMismatch { expected: u32, found: u32 },
    DriverUnavailable,
    DriverProtocolMismatch { expected: u32, found: u32 },
}

/// Validates the two independent Dokany version domains without loading any
/// Windows components, so compatibility policy can be tested cross-platform.
pub(crate) const fn validate_dokany_version_domains(
    library_api: u32,
    driver_protocol: u32,
) -> Result<(), DokanyVersionCompatibilityError> {
    if library_api != DOKANY_LIBRARY_API_VERSION {
        return Err(DokanyVersionCompatibilityError::LibraryApiMismatch {
            expected: DOKANY_LIBRARY_API_VERSION,
            found: library_api,
        });
    }
    if driver_protocol == 0 {
        return Err(DokanyVersionCompatibilityError::DriverUnavailable);
    }
    if driver_protocol != DOKANY_DRIVER_PROTOCOL_VERSION {
        return Err(DokanyVersionCompatibilityError::DriverProtocolMismatch {
            expected: DOKANY_DRIVER_PROTOCOL_VERSION,
            found: driver_protocol,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct DriveRuntimeInfo {
    /// Legacy machine-output field retained for patch-release compatibility.
    pub required_api: u32,
    pub required_library_api: u32,
    pub library_api: u32,
    /// Legacy name retained for compatibility; this value is the driver
    /// protocol revision returned by `DokanDriverVersion()`.
    pub driver_api: u32,
    pub required_driver_protocol: u32,
    pub driver_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DriveRuntimeInstallOutcome {
    AlreadyReady,
    Installed { code: u32 },
    RebootRequired { code: u32 },
    RestartInitiated { code: u32 },
    Cancelled { code: u32 },
    AnotherInstallationRunning { code: u32 },
    FatalInstallerError { code: u32 },
    UnsupportedPlatform { code: u32 },
    UnexpectedFailure { code: u32 },
}

impl DriveRuntimeInstallOutcome {
    pub(crate) const fn from_msi_exit_code(code: u32) -> Self {
        match code {
            0 => Self::Installed { code: 0 },
            3010 => Self::RebootRequired { code: 3010 },
            1641 => Self::RestartInitiated { code: 1641 },
            1223 | 1602 => Self::Cancelled { code },
            1618 => Self::AnotherInstallationRunning { code: 1618 },
            1603 => Self::FatalInstallerError { code: 1603 },
            1633 | 1654 => Self::UnsupportedPlatform { code },
            code => Self::UnexpectedFailure { code },
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::AlreadyReady | Self::Installed { .. } => 0,
            Self::RebootRequired { code }
            | Self::RestartInitiated { code }
            | Self::Cancelled { code }
            | Self::AnotherInstallationRunning { code }
            | Self::FatalInstallerError { code }
            | Self::UnsupportedPlatform { code } => *code as i32,
            Self::UnexpectedFailure { .. } => 1,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::AlreadyReady => format!(
                "Dokany 2.3.1 (DLL-API {DOKANY_LIBRARY_API_VERSION}, Treiberprotokoll {DOKANY_DRIVER_PROTOCOL_VERSION}) ist bereits einsatzbereit."
            ),
            Self::Installed { .. } => format!(
                "Dokany 2.3.1 (DLL-API {DOKANY_LIBRARY_API_VERSION}, Treiberprotokoll {DOKANY_DRIVER_PROTOCOL_VERSION}) wurde installiert und ist einsatzbereit."
            ),
            Self::RebootRequired { .. } => {
                "Dokany wurde installiert. Windows muss neu gestartet werden.".into()
            }
            Self::RestartInitiated { .. } => {
                "Dokany wurde installiert; Windows hat den Neustart eingeleitet.".into()
            }
            Self::Cancelled { .. } => "Dokany-Installation wurde abgebrochen.".into(),
            Self::AnotherInstallationRunning { .. } => {
                "Eine andere Windows-Installation laeuft bereits (MSI 1618).".into()
            }
            Self::FatalInstallerError { .. } => {
                "Dokany-Installation ist mit einem schwerwiegenden MSI-Fehler fehlgeschlagen (1603)."
                    .into()
            }
            Self::UnsupportedPlatform { code } => format!(
                "Diese Dokany-Laufzeit wird von diesem Windows-System nicht unterstuetzt (MSI {code})."
            ),
            Self::UnexpectedFailure { code } => {
                format!("Dokany-Installation ist mit MSI-Code {code} fehlgeschlagen.")
            }
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::AnotherInstallationRunning { .. }
                | Self::FatalInstallerError { .. }
                | Self::UnsupportedPlatform { .. }
                | Self::UnexpectedFailure { .. }
        )
    }
}
