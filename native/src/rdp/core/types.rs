use serde::{Deserialize, Serialize};

pub(crate) const RDP_PROBE_VERSION: u16 = 1;
/// Probe v2 (0.5.189): adds the SHA-1 listener thumbprint and se-managed
/// service credentials; a v1 request still receives exactly the v1 form.
pub(crate) const RDP_PROBE_VERSION_V2: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostKind {
    WindowsWorkstation,
    WindowsServer,
    Gnome,
    Krdp,
    Xrdp,
}

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
    /// Service rank of design 2.3: the smallest rank wins among startable
    /// services. A shared console first („Bildschirmfreigabe heißt
    /// Bildschirmfreigabe“), a session that locks the local screen last.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::SharedConsole => 1,
            Self::AdditionalConsoleDisplay => 2,
            Self::HeadlessUserSession => 3,
            Self::SeparateXorg => 4,
            Self::SeparateXvnc => 5,
            Self::WindowsServerSession | Self::RemoteLogin => 6,
            Self::WindowsConsoleLocked | Self::ExclusiveConsole => 7,
            Self::Unknown => u8::MAX,
        }
    }
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
        if value.len() > 95 {
            return Err("RDP SHA-256 fingerprint exceeds canonical digest length".into());
        }
        let normalized: String = value
            .chars()
            .filter(|c| *c == ':' || c.is_ascii_hexdigit())
            .filter(|c| *c != ':')
            .collect();
        if value.chars().any(|c| c != ':' && !c.is_ascii_hexdigit()) || normalized.len() != 64 {
            return Err("RDP certificate fingerprint must be exactly one SHA-256 digest".into());
        }
        Ok(Self(normalized.to_ascii_uppercase()))
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for CertificateFingerprint {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}
impl From<CertificateFingerprint> for String {
    fn from(value: CertificateFingerprint) -> Self {
        value.0
    }
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
        !alias.is_empty()
            && alias.len() <= 256
            && alias
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
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
    /// Probe v2: SHA-1 thumbprint of the listener certificate (the value
    /// mstsc stores as `CertHash`). Absent in every v1 answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) certificate_sha1: Option<CertificateSha1>,
    /// Probe v2: se created this service's own RDP credentials (GNOME
    /// desktop sharing) and hands them out through `RdpCredential`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) managed_credentials: bool,
    /// Host-local only: the process that owned the listener when the probe
    /// proved it (design 2.7 point 4). Never sent over the wire.
    #[serde(skip)]
    pub(crate) listener_pid: Option<u32>,
}
fn is_false(value: &bool) -> bool {
    !*value
}
impl HostService {
    pub(crate) fn startable(&self) -> Result<(), String> {
        if let Some(reason) = &self.unavailable_reason {
            return Err(reason.clone());
        }
        if !self.running || self.port.is_none_or(|port| port == 0) {
            return Err("RDP service has no proved active TCP listener".into());
        }
        if self.session_mode == SessionMode::Unknown {
            return Err(self.session_mode.notice().into());
        }
        if self.tls != Some(true) {
            return Err("TLS support of the RDP service is not proved".into());
        }
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
        Self {
            version: RDP_PROBE_VERSION,
            platform: platform.into(),
            services: Vec::new(),
            unavailable_reason: Some(reason.into()),
            diagnostics: Vec::new(),
        }
    }
    /// The startable service of the smallest rank (design 2.3); a tie goes
    /// to the smaller service ID.
    pub(crate) fn preferred_service(&self) -> Option<&HostService> {
        self.services
            .iter()
            .filter(|service| service.startable().is_ok())
            .min_by_key(|service| (service.session_mode.rank(), service.id.clone()))
    }
    /// The exact v1 form for a host answering an old `RdpProbe`.
    pub(crate) fn into_v1(mut self) -> Self {
        self.version = RDP_PROBE_VERSION;
        for service in &mut self.services {
            service.certificate_sha1 = None;
            service.managed_credentials = false;
        }
        self
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        let v2_fields = self
            .services
            .iter()
            .any(|s| s.certificate_sha1.is_some() || s.managed_credentials);
        if !matches!(self.version, RDP_PROBE_VERSION | RDP_PROBE_VERSION_V2)
            || (self.version == RDP_PROBE_VERSION && v2_fields)
            || self.services.len() > 32
            || self.platform.len() > 64
            || self.diagnostics.len() > 128
            || self.diagnostics.iter().any(|s| s.len() > 4096)
            || self
                .unavailable_reason
                .as_ref()
                .is_some_and(|s| s.len() > 4096)
        {
            return Err("Unsupported or oversized RDP host inventory".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for service in &self.services {
            if !PublishedApp::valid_alias(&service.id)
                || !ids.insert(&service.id)
                || service.remote_apps.len() > 1024
                || service.evidence.len() > 128
                || service.evidence.iter().any(|s| s.len() > 4096)
                || service.version.as_ref().is_some_and(|s| s.len() > 256)
                || service
                    .certificate_source
                    .as_ref()
                    .is_some_and(|s| s.len() > 4096)
                || service
                    .unavailable_reason
                    .as_ref()
                    .is_some_and(|s| s.len() > 4096)
            {
                return Err("Invalid RDP service inventory".into());
            }
            let mode_matches = match service.kind {
                HostKind::WindowsWorkstation => {
                    service.session_mode == SessionMode::WindowsConsoleLocked
                }
                HostKind::WindowsServer => {
                    service.session_mode == SessionMode::WindowsServerSession
                }
                HostKind::Gnome => matches!(
                    service.session_mode,
                    SessionMode::SharedConsole
                        | SessionMode::HeadlessUserSession
                        | SessionMode::RemoteLogin
                        | SessionMode::Unknown
                ),
                HostKind::Krdp => matches!(
                    service.session_mode,
                    SessionMode::SharedConsole
                        | SessionMode::ExclusiveConsole
                        | SessionMode::AdditionalConsoleDisplay
                        | SessionMode::Unknown
                ),
                HostKind::Xrdp => matches!(
                    service.session_mode,
                    SessionMode::SeparateXorg | SessionMode::SeparateXvnc | SessionMode::Unknown
                ),
            };
            if !mode_matches
                || service.certificate_sha256.is_some()
                    && service
                        .certificate_source
                        .as_ref()
                        .is_none_or(|s| s.is_empty())
            {
                return Err(
                    "RDP session mode or certificate provenance contradicts the service inventory"
                        .into(),
                );
            }
            if !service.remote_apps.is_empty() && service.kind != HostKind::WindowsServer {
                return Err("RemoteApp requires a proved Windows Server RDS inventory".into());
            }
            let mut aliases = std::collections::BTreeSet::new();
            if service.remote_apps.iter().any(|app| {
                !PublishedApp::valid_alias(&app.alias)
                    || !aliases.insert(&app.alias)
                    || app.collection.is_empty()
                    || app.collection.len() > 256
                    || app.display_name.len() > 1024
            }) {
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
pub(crate) enum ClientKind {
    Mstsc,
    FreeRdp3,
}
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

/// SHA-1 thumbprint of the RDP listener certificate (mstsc's `CertHash`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct CertificateSha1(String);
impl CertificateSha1 {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let normalized: String = value.chars().filter(|c| *c != ':' && *c != ' ').collect();
        if normalized.len() != 40 || !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("RDP certificate thumbprint must be exactly one SHA-1 digest".into());
        }
        Ok(Self(normalized.to_ascii_uppercase()))
    }
    #[allow(dead_code)]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
    /// The 20 bytes of the digest (`CertHash` is REG_BINARY).
    #[allow(dead_code)] // Consumer: the viewer's mstsc `CertHash` (WP2).
    pub(crate) fn bytes(&self) -> [u8; 20] {
        let mut out = [0u8; 20];
        for (index, byte) in out.iter_mut().enumerate() {
            let pair = &self.0[index * 2..index * 2 + 2];
            *byte = u8::from_str_radix(pair, 16).unwrap_or_default();
        }
        out
    }
}
impl TryFrom<String> for CertificateSha1 {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}
impl From<CertificateSha1> for String {
    fn from(value: CertificateSha1) -> Self {
        value.0
    }
}

/// What starts the host's RDP self-setup (design 3.1, T1–T4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SetupTrigger {
    /// T1: a relation received „Bildschirm steuern: Erlauben“.
    Grant,
    /// T2: first start of this version with such a relation.
    VersionStart,
    /// T3: a device with this right became present.
    PeerOnline,
    /// T4: a viewer clicked „Remote Desktop“ (`RdpPrepare`).
    PeerRequest,
}
impl SetupTrigger {
    /// A person acted: a declined or failed setup may ask again.
    pub(crate) fn user_action(self) -> bool {
        matches!(self, Self::Grant | Self::PeerRequest)
    }
}

/// Result of the host's RDP self-setup; the marker file and the probe v2
/// field `setup` carry it as `done`, `declined`, `not_needed`,
/// `impossible:<reason>` or `failed:<reason>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) enum SetupState {
    Done,
    Declined,
    Impossible(String),
    Failed(String),
    NotNeeded,
}
impl SetupState {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        if value.len() > 1024 || value.chars().any(char::is_control) {
            return Err("RDP setup state is oversized or contains control characters".into());
        }
        Ok(match value {
            "done" => Self::Done,
            "declined" => Self::Declined,
            "not_needed" => Self::NotNeeded,
            other => match other.split_once(':') {
                Some(("impossible", reason)) if !reason.is_empty() => {
                    Self::Impossible(reason.into())
                }
                Some(("failed", reason)) if !reason.is_empty() => Self::Failed(reason.into()),
                _ => return Err(format!("unknown RDP setup state {other:?}")),
            },
        })
    }
    /// A later setup may still make Remote Desktop available.
    #[allow(dead_code)] // Consumers: the viewer's RDP entries (WP2/WP3).
    pub(crate) fn setup_possible(&self) -> bool {
        !matches!(self, Self::Impossible(_))
    }
    /// The German text for every surface (design 3.4).
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Done => "Remote Desktop ist eingerichtet".into(),
            Self::NotNeeded => "Remote Desktop war bereits verfügbar".into(),
            Self::Declined => "Administratorfreigabe für Remote Desktop wurde abgelehnt; erneut bei der nächsten Freigabe oder beim Klick „Remote Desktop“".into(),
            Self::Impossible(reason) => match reason.as_str() {
                "windows_home" => "Windows Home kann keinen Remote-Desktop-Zugang anbieten; Bildschirm ansehen/steuern bleibt verfügbar".into(),
                "policy_denies_rdp" => "Eine Gruppenrichtlinie verbietet Remote Desktop auf diesem Gerät".into(),
                "krdp_no_users" => "KRdp: in den Systemeinstellungen unter Remote Desktop einen Benutzer anlegen".into(),
                "android" => "Android bietet keinen Remote-Desktop-Dienst an; Bildschirm ansehen/steuern bleibt verfügbar".into(),
                "se_binary_not_root_owned" => "se ist nicht als Systemprogramm installiert (Datei und Ordner müssen root gehören); Remote Desktop kann nicht eingerichtet werden".into(),
                other => format!("Remote Desktop ist hier nicht einrichtbar: {other}"),
            },
            Self::Failed(reason) => match reason.as_str() {
                "no_interactive_session" => "Keine angemeldete Sitzung für die Administratorfreigabe; erneut, sobald ein Gerät mit dieser Freigabe erscheint".into(),
                "no_polkit_agent" => "Kein polkit-Dialog verfügbar; Remote Desktop kann nicht eingerichtet werden".into(),
                "keyring_locked" => "GNOME-Schlüsselbund ist gesperrt; erneut, sobald ein Gerät mit dieser Freigabe erscheint".into(),
                "not_set_up" => "Remote Desktop ist noch nicht eingerichtet; „Remote Desktop“ richtet es ein".into(),
                other => format!("Einrichtung von Remote Desktop fehlgeschlagen: {other}"),
            },
        }
    }
}
impl std::fmt::Display for SetupState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Done => f.write_str("done"),
            Self::Declined => f.write_str("declined"),
            Self::NotNeeded => f.write_str("not_needed"),
            Self::Impossible(reason) => write!(f, "impossible:{reason}"),
            Self::Failed(reason) => write!(f, "failed:{reason}"),
        }
    }
}
impl TryFrom<String> for SetupState {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}
impl From<SetupState> for String {
    fn from(value: SetupState) -> Self {
        value.to_string()
    }
}

/// The RDP short state of the device feed (design 9.4 `rdp`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RdpFeedSummary {
    pub(crate) startable: bool,
    pub(crate) setup: SetupState,
    pub(crate) remote_apps: Vec<PublishedApp>,
}
impl RdpFeedSummary {
    pub(crate) fn from_probe(probe: &HostProbe, setup: SetupState) -> Self {
        let preferred = probe.preferred_service();
        Self {
            startable: preferred.is_some(),
            setup,
            remote_apps: preferred
                .map(|service| service.remote_apps.clone())
                .unwrap_or_default(),
        }
    }
}

/// Last probe/setup result of this host for the device feed (design 5.2).
static HOST_SUMMARY: std::sync::RwLock<Option<RdpFeedSummary>> = std::sync::RwLock::new(None);

pub(crate) fn note_host_summary(summary: RdpFeedSummary) {
    if let Ok(mut current) = HOST_SUMMARY.write() {
        *current = Some(summary);
    }
}

pub(crate) fn host_summary() -> Option<RdpFeedSummary> {
    HOST_SUMMARY.read().ok().and_then(|current| current.clone())
}
