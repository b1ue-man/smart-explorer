use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use super::cache_policy::{MountCachePolicy, MountRuntimePreference};
use super::metadata_policy::MountMetadataPolicy;
use super::path_dialect::PathDialect;
use super::range_read::RangeReadPolicy;

// Moved without any change of the serde form (DRV-M1); re-exported so every
// caller keeps the old path.
pub use super::mount_source::{MountSource, PeerMountTarget};
pub use super::mount_status::{FlushOutcome, MountStatus, NamespaceOutcome, RenameOutcome};

// DRV-M3: the folder a drive appears at. Declared here because `mount/mod.rs`
// belongs to the hub of the mount module; `MountConfig` offers the accessors
// that callers outside `crate::mount` need.
#[path = "mount_location.rs"]
mod mount_location;
pub use mount_location::MountLocation;

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

    /// The root `/` of a source without a choosable root (`Devices`,
    /// `Vault`).
    pub(crate) fn top() -> Self {
        Self("/".to_string())
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
    #[serde(default)]
    pub cache: MountCachePolicy,
    #[serde(default)]
    pub runtime_preference: MountRuntimePreference,
    pub label: String,
    /// DRV-M3: the folder the drive appears at; absent means the default of
    /// the platform. Written only when set, so earlier entries keep their
    /// bytes and older versions ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<MountLocation>,
    /// DRV-M3: „Beim Anmelden automatisch einbinden“. Written only when on.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_remount: bool,
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
            cache: MountCachePolicy::default(),
            runtime_preference: MountRuntimePreference::default(),
            label: label.into(),
            location: None,
            auto_remount: false,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> io::Result<()> {
        self.source.validate()?;
        self.metadata.validate()?;
        self.cache.validate()?;
        if let Some(location) = &self.location {
            location.validate()?;
        }
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
            cache: self.cache,
            runtime_preference: self.runtime_preference,
            path_dialect: PathDialect::Windows,
            range_reads: RangeReadPolicy::default(),
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

    pub fn with_cache_policy(mut self, cache: MountCachePolicy) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_runtime_preference(mut self, preference: MountRuntimePreference) -> Self {
        self.runtime_preference = preference;
        self
    }

    pub fn with_auto_remount(mut self, auto_remount: bool) -> Self {
        self.auto_remount = auto_remount;
        self
    }

    /// Sets (`Some`) or clears (`None`) the folder the drive appears at.
    pub fn with_location_directory(mut self, path: Option<PathBuf>) -> io::Result<Self> {
        self.location = path.map(MountLocation::directory).transpose()?;
        Ok(self)
    }

    /// The folder the user chose for the drive, if any.
    pub fn location_directory(&self) -> Option<&Path> {
        self.location.as_ref().map(MountLocation::path)
    }

    /// Whether `path` is acceptable as a drive folder (form only).
    pub fn check_location_directory(path: &Path) -> io::Result<()> {
        MountLocation::directory(path).map(|_| ())
    }

    /// Whether the drive is mounted again at login and after a worker
    /// restart (DRV-M3): „Beim Anmelden automatisch einbinden“, or the device
    /// tree „Verbundene Geräte als Ordner“, which stays on until switched off
    /// (strand S2 C2 and C5).
    pub fn restores_at_login(&self) -> bool {
        self.auto_remount || self.source.is_device_tree()
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Sanitized configuration passed to a filesystem host. The host receives an
/// already-rooted Backend and therefore never needs account or peer identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountRuntimeConfig {
    pub id: MountId,
    pub mode: MountMode,
    #[serde(default)]
    pub metadata: MountMetadataPolicy,
    #[serde(default)]
    pub cache: MountCachePolicy,
    #[serde(default)]
    pub runtime_preference: MountRuntimePreference,
    /// Name rules of the serving frontend (DRV-M1). Absent means Windows, the
    /// only rules before the field existed; the Windows value is not written,
    /// so earlier bytes stay unchanged.
    #[serde(default, skip_serializing_if = "PathDialect::is_windows")]
    pub path_dialect: PathDialect,
    /// Which handles read byte ranges without a whole-file copy (DRV-M8).
    /// Absent means read-only drives only; that value is not written.
    #[serde(default, skip_serializing_if = "RangeReadPolicy::is_default")]
    pub range_reads: RangeReadPolicy,
}

impl MountRuntimeConfig {
    pub fn new(id: MountId, mode: MountMode) -> Self {
        Self {
            id,
            mode,
            metadata: MountMetadataPolicy::default(),
            cache: MountCachePolicy::default(),
            runtime_preference: MountRuntimePreference::default(),
            path_dialect: PathDialect::Windows,
            range_reads: RangeReadPolicy::default(),
        }
    }

    /// The Android documents provider (DRV-M8): every read-only handle of a
    /// writable root reads large files by ranges.
    pub fn with_range_reads_on_read_only_handles(mut self) -> Self {
        self.range_reads = RangeReadPolicy::ReadOnlyHandles;
        self
    }

    pub fn with_path_dialect(mut self, dialect: PathDialect) -> Self {
        self.path_dialect = dialect;
        self
    }

    pub fn with_metadata_policy(mut self, metadata: MountMetadataPolicy) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_cache_policy(mut self, cache: MountCachePolicy) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_runtime_preference(mut self, preference: MountRuntimePreference) -> Self {
        self.runtime_preference = preference;
        self
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeleteToken(pub(crate) u64);

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
