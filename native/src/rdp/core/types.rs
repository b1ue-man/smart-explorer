use serde::{Deserialize, Serialize};

pub(crate) const RDP_PROBE_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostKind { WindowsWorkstation, WindowsServer, Gnome, Krdp, Xrdp }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionMode {
    WindowsConsoleLocked,
    WindowsServerSession,
    SharedConsole,
    ExclusiveConsole,
    AdditionalConsoleDisplay,
    HeadlessUserSession,
    RemoteLogin,
    SeparateXorg,
    SeparateXvnc,
    Unknown,
}
impl SessionMode {
    pub(crate) fn notice(self) -> &'static str {
        match self {
            Self::WindowsConsoleLocked => "Windows sperrt den lokalen Bildschirm; der angemeldete Benutzer kann getrennt werden.",
            Self::WindowsServerSession => "Windows Server verbindet eine RDS-Sitzung; vorhandene Sitzungen können gemäß der Serverrichtlinie wiederverbunden werden.",
            Self::SharedConsole => "Der sichtbare lokale Desktop wird geteilt; die lokale Person kann die Fernbedienung sehen.",
            Self::ExclusiveConsole => "KRdp erstellt einen virtuellen Bildschirm in der bestehenden Sitzung, fordert lokal den Wechsel zum Anmeldebildschirm an und entsperrt die entfernte Sitzung. Beim Ende der letzten RDP-Verbindung wird die Sitzung gesperrt; dies ist keine neue Anmeldung.",
            Self::AdditionalConsoleDisplay => "KRdp erstellt einen zusätzlichen virtuellen Bildschirm in der bestehenden Benutzersitzung; dies ist keine separate Anmeldung.",
            Self::HeadlessUserSession => "GNOME verwendet eine bildschirmlose Benutzersitzung, nicht den sichtbaren lokalen Desktop.",
            Self::RemoteLogin => "GNOME bietet Remote Login mit Anmeldung an einer entfernten Sitzung; bestehende Sitzungen unterliegen der GDM-Richtlinie.",
            Self::SeparateXorg => "xrdp meldet an einer separaten Xorg-Sitzung an oder verbindet eine bestehende xrdp-Sitzung wieder.",
            Self::SeparateXvnc => "xrdp meldet an einer separaten Xvnc-Sitzung an oder verbindet eine bestehende xrdp-Sitzung wieder.",
            Self::Unknown => "Die konkrete Sitzungswirkung ist nicht belegt; ein Start wird abgewiesen.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct CertificateFingerprint(String);
impl CertificateFingerprint {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        if value.len() > 95 { return Err("RDP SHA-256 fingerprint exceeds canonical digest length".into()); }
        let normalized: String = value.chars().filter(|c| *c == ':' || c.is_ascii_hexdigit())
            .filter(|c| *c != ':').collect();
        if value.chars().any(|c| c != ':' && !c.is_ascii_hexdigit()) || normalized.len() != 64 {
            return Err("RDP certificate fingerprint must be exactly one SHA-256 digest".into());
        }
        Ok(Self(normalized.to_ascii_uppercase()))
    }
    pub(crate) fn as_str(&self) -> &str { &self.0 }
}
impl TryFrom<String> for CertificateFingerprint {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> { Self::parse(&value) }
}
impl From<CertificateFingerprint> for String {
    fn from(value: CertificateFingerprint) -> Self { value.0 }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishedApp {
    pub(crate) alias: String,
    pub(crate) display_name: String,
    pub(crate) collection: String,
}
impl PublishedApp {
    pub(crate) fn valid_alias(alias: &str) -> bool {
        !alias.is_empty() && alias.len() <= 256
            && alias.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostService {
    pub(crate) id: String,
    pub(crate) kind: HostKind,
    pub(crate) version: Option<String>,
    pub(crate) session_mode: SessionMode,
    /// Effective listener port, proved to belong to the service process.
    pub(crate) port: Option<u16>,
    pub(crate) running: bool,
    pub(crate) tls: Option<bool>,
    pub(crate) nla: Option<bool>,
    pub(crate) certificate_sha256: Option<CertificateFingerprint>,
    pub(crate) certificate_source: Option<String>,
    /// Only Windows Server + locally assigned RDS session collections.
    pub(crate) remote_apps: Vec<PublishedApp>,
    pub(crate) evidence: Vec<String>,
    pub(crate) unavailable_reason: Option<String>,
}
impl HostService {
    pub(crate) fn startable(&self) -> Result<(), String> {
        if let Some(reason) = &self.unavailable_reason { return Err(reason.clone()); }
        if !self.running || self.port.is_none_or(|port| port == 0) {
            return Err("RDP service has no proved active TCP listener".into());
        }
        if self.session_mode == SessionMode::Unknown { return Err(self.session_mode.notice().into()); }
        if self.tls != Some(true) { return Err("TLS support of the RDP service is not proved".into()); }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostProbe {
    pub(crate) version: u16,
    pub(crate) platform: String,
    pub(crate) services: Vec<HostService>,
    pub(crate) unavailable_reason: Option<String>,
    pub(crate) diagnostics: Vec<String>,
}
impl HostProbe {
    pub(crate) fn unavailable(platform: &str, reason: &str) -> Self {
        Self { version: RDP_PROBE_VERSION, platform: platform.into(), services: Vec::new(),
            unavailable_reason: Some(reason.into()), diagnostics: Vec::new() }
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != RDP_PROBE_VERSION || self.services.len() > 32 || self.platform.len() > 64
            || self.diagnostics.len() > 128 || self.diagnostics.iter().any(|s| s.len() > 4096)
            || self.unavailable_reason.as_ref().is_some_and(|s| s.len() > 4096) {
            return Err("Unsupported or oversized RDP host inventory".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for service in &self.services {
            if !PublishedApp::valid_alias(&service.id) || !ids.insert(&service.id)
                || service.remote_apps.len() > 1024 || service.evidence.len() > 128
                || service.evidence.iter().any(|s| s.len() > 4096)
                || service.version.as_ref().is_some_and(|s| s.len() > 256)
                || service.certificate_source.as_ref().is_some_and(|s| s.len() > 4096)
                || service.unavailable_reason.as_ref().is_some_and(|s| s.len() > 4096) {
                return Err("Invalid RDP service inventory".into());
            }
            let mode_matches = match service.kind {
                HostKind::WindowsWorkstation => service.session_mode == SessionMode::WindowsConsoleLocked,
                HostKind::WindowsServer => service.session_mode == SessionMode::WindowsServerSession,
                HostKind::Gnome => matches!(service.session_mode, SessionMode::SharedConsole | SessionMode::HeadlessUserSession | SessionMode::RemoteLogin | SessionMode::Unknown),
                HostKind::Krdp => matches!(service.session_mode, SessionMode::SharedConsole | SessionMode::ExclusiveConsole | SessionMode::AdditionalConsoleDisplay | SessionMode::Unknown),
                HostKind::Xrdp => matches!(service.session_mode, SessionMode::SeparateXorg | SessionMode::SeparateXvnc | SessionMode::Unknown),
            };
            if !mode_matches || service.certificate_sha256.is_some() && service.certificate_source.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("RDP session mode or certificate provenance contradicts the service inventory".into());
            }
            if !service.remote_apps.is_empty() && service.kind != HostKind::WindowsServer {
                return Err("RemoteApp requires a proved Windows Server RDS inventory".into());
            }
            let mut aliases = std::collections::BTreeSet::new();
            if service.remote_apps.iter().any(|app| !PublishedApp::valid_alias(&app.alias)
                || !aliases.insert(&app.alias) || app.collection.is_empty()
                || app.collection.len() > 256 || app.display_name.len() > 1024) {
                return Err("Invalid or ambiguous published RemoteApp alias".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CertificatePolicy {
    /// No currently supported external client proves this exclusive policy.
    ExactPeerPin,
    /// FreeRDP's documented pin acceptance OR normal native certificate trust.
    PeerPinOrNativeTrust,
    /// Human compares the peer fingerprint in the existing native dialog.
    NativeDialog,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientKind { Mstsc, FreeRdp3 }
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ClientCapability {
    pub(crate) id: String,
    pub(crate) kind: ClientKind,
    pub(crate) version: Option<String>,
    pub(crate) certificate_policy: CertificatePolicy,
    pub(crate) remote_app: bool,
    pub(crate) available: bool,
    pub(crate) detail: String,
}
