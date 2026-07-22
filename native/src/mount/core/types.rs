use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;
use std::io;

use super::metadata_policy::MountMetadataPolicy;

const MAX_ID_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct MountId(String);

impl MountId {
    pub fn new_random() -> io::Result<Self> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
        let mut value = String::with_capacity(32);
        for byte in random {
            use fmt::Write as _;
            let _ = write!(value, "{byte:02x}");
        }
        Ok(Self(value))
    }

    pub fn parse(value: &str) -> io::Result<Self> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(invalid("mount id must be 1-64 ASCII identifier characters"));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for MountId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

/// A canonical forward-slash root inside one already-authorized backend.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct BackendRoot(String);

impl BackendRoot {
    pub fn parse(value: &str) -> io::Result<Self> {
        if value.is_empty()
            || value.encode_utf16().count() > 32_767
            || value.contains('\0')
            || value.contains('\\')
        {
            return Err(invalid(
                "backend root must be a bounded absolute forward-slash path",
            ));
        }
        if value == "/" {
            return Ok(Self("/".to_string()));
        }
        let unc = value.starts_with("//");
        if !value.starts_with('/') || value.starts_with("///") {
            return Err(invalid("backend root must start with '/'"));
        }
        let prefix_len = if unc { 2 } else { 1 };
        let mut components = Vec::new();
        for component in value[prefix_len..].split('/') {
            if component.is_empty() {
                return Err(invalid("backend root contains an empty path component"));
            }
            if matches!(component, "." | "..") {
                return Err(invalid("backend root may not contain '.' or '..'"));
            }
            components.push(component);
        }
        if unc && components.len() < 2 {
            return Err(invalid("UNC backend root requires server and share names"));
        }
        let canonical = format!("{}{}", if unc { "//" } else { "/" }, components.join("/"));
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BackendRoot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerMountTarget {
    Direct { contact_id: String },
    RoomDevice { room_id: String, device_id: String },
}

impl PeerMountTarget {
    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::Direct { contact_id } => validate_identity(contact_id, "contact id"),
            Self::RoomDevice { room_id, device_id } => {
                validate_identity(room_id, "room id")?;
                validate_identity(device_id, "device id")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountSource {
    SavedRemote {
        account: String,
        root: BackendRoot,
    },
    GoogleDrive {
        account: String,
        root: BackendRoot,
    },
    Peer {
        target: PeerMountTarget,
        root: BackendRoot,
    },
}

impl MountSource {
    pub fn root(&self) -> &BackendRoot {
        match self {
            Self::SavedRemote { root, .. }
            | Self::GoogleDrive { root, .. }
            | Self::Peer { root, .. } => root,
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::SavedRemote { account, .. } => validate_identity(account, "saved account"),
            Self::GoogleDrive { account, .. } => {
                validate_identity(account, "Google Drive account")?;
                if account != "cloud:gdrive" {
                    return Err(invalid("unsupported Google Drive account identity"));
                }
                Ok(())
            }
            Self::Peer { target, .. } => target.validate(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct DriveLetter(char);

impl DriveLetter {
    pub fn parse(value: char) -> io::Result<Self> {
        if !value.is_ascii_alphabetic() {
            return Err(invalid("drive letter must be an ASCII letter"));
        }
        Ok(Self(value.to_ascii_uppercase()))
    }

    pub fn get(self) -> char {
        self.0
    }
}

impl fmt::Display for DriveLetter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:", self.0)
    }
}

impl<'de> Deserialize<'de> for DriveLetter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let mut chars = value.chars();
        let letter = chars
            .next()
            .ok_or_else(|| de::Error::custom("empty drive letter"))?;
        if chars.next().is_some() {
            return Err(de::Error::custom("drive letter must contain one character"));
        }
        Self::parse(letter).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriveSelection {
    Automatic,
    Letter(DriveLetter),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountMode {
    ReadOnly,
    ReadWrite,
}

/// Whether the selected backend must technically confine every resolved path
/// to the chosen root, or whether the user explicitly trusts the server and
/// concurrent writers while Smart Explorer applies serialized path checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountRootSecurity {
    #[default]
    Enforced,
    Trusted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountConfig {
    pub id: MountId,
    pub source: MountSource,
    pub drive: DriveSelection,
    pub mode: MountMode,
    #[serde(default)]
    pub root_security: MountRootSecurity,
    #[serde(default)]
    pub metadata: MountMetadataPolicy,
    pub label: String,
}

impl MountConfig {
    pub fn new(
        id: MountId,
        source: MountSource,
        drive: DriveSelection,
        mode: MountMode,
        label: impl Into<String>,
    ) -> io::Result<Self> {
        let config = Self {
            id,
            source,
            drive,
            mode,
            root_security: MountRootSecurity::Enforced,
            metadata: MountMetadataPolicy::default(),
            label: label.into(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> io::Result<()> {
        self.source.validate()?;
        self.metadata.validate()?;
        if self.label.chars().count() > 128
            || self
                .label
                .chars()
                .any(|character| character == '\0' || character.is_control())
        {
            return Err(invalid("mount label is invalid or too long"));
        }
        Ok(())
    }

    pub fn runtime(&self) -> MountRuntimeConfig {
        MountRuntimeConfig {
            id: self.id.clone(),
            mode: self.mode,
            metadata: self.metadata,
        }
    }

    pub fn with_root_security(mut self, root_security: MountRootSecurity) -> Self {
        self.root_security = root_security;
        self
    }

    pub fn with_metadata_policy(mut self, metadata: MountMetadataPolicy) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Sanitized configuration passed to a filesystem host. The host receives an
/// already-rooted Backend and therefore never needs account or peer identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountRuntimeConfig {
    pub id: MountId,
    pub mode: MountMode,
    #[serde(default)]
    pub metadata: MountMetadataPolicy,
}

impl MountRuntimeConfig {
    pub fn new(id: MountId, mode: MountMode) -> Self {
        Self {
            id,
            mode,
            metadata: MountMetadataPolicy::default(),
        }
    }

    pub fn with_metadata_policy(mut self, metadata: MountMetadataPolicy) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountStatus {
    Unmounted,
    Mounting,
    Mounted {
        drive: DriveLetter,
    },
    Unmounting,
    RuntimeUnavailable {
        detail: String,
    },
    Conflict {
        drive: DriveLetter,
        path: String,
        detail: String,
    },
    Failed {
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HandleId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenDisposition {
    OpenExisting,
    OpenOrCreate,
    CreateNew,
    TruncateExisting,
    CreateAlways,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenFileOptions {
    pub writable: bool,
    pub disposition: OpenDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Baseline {
    Missing,
    Present {
        id: Option<String>,
        size: u64,
        mtime_ms: i64,
        #[serde(default)]
        content_md5: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountConflict {
    pub path: String,
    pub baseline: Baseline,
    pub current: Option<Baseline>,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum NamespaceOperation {
    CreateDirectory,
    RenameNoReplace,
}

/// Durable evidence written before a namespace mutation is dispatched.  The
/// structured source fields let Retry reconcile a lost reply without parsing
/// a human-readable error or replaying the mutation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct NamespaceIntent {
    pub conflict: MountConflict,
    pub operation: NamespaceOperation,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub source_baseline: Option<Baseline>,
    #[serde(default)]
    pub source_is_directory: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryCondition {
    Clean,
    Dirty,
    Conflict(MountConflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlushOutcome {
    NoChanges,
    Committed,
    /// The backend acknowledged the atomic promotion, but the new metadata or
    /// local journal could not be reconciled. Callers must return filesystem
    /// success (the namespace already changed) and surface this recovery state.
    CommittedPendingVerification(MountConflict),
    Conflict(MountConflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameOutcome {
    Complete,
    /// The namespace change is already committed. Filesystem callers must see
    /// success while Smart Explorer surfaces the retained recovery state.
    CommittedPendingVerification(MountConflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceOutcome {
    Complete,
    /// A namespace mutation was dispatched or acknowledged, but its final
    /// shape could not be verified. Windows must not replay it; the host stops
    /// and remounts after surfacing the detail.
    CommittedPendingVerification {
        path: String,
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeleteToken(pub(crate) u64);

fn validate_identity(value: &str, name: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID_LEN
        || value
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(invalid(format!("invalid {name}")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
