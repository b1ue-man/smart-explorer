use std::path::PathBuf;

use super::hash::normalize_sha256;

const CURRENT_PROTOCOL_ARGS: &[&str] = &[
    "--target-sha256",
    "--helper-target",
    "--cli-staged",
    "--cli-target",
    "--cli-sha256",
    "--archive",
    "--manifest",
    "--pin-file",
];
const CURRENT_VALUE_ARGS: &[&str] = &[
    "--target",
    "--target-sha256",
    "--staged",
    "--staged-sha256",
    "--helper-target",
    "--helper-sha256",
    "--cli-staged",
    "--cli-target",
    "--cli-sha256",
    "--archive",
    "--parent-pid",
    "--version",
    "--last-applied",
    "--error-file",
    "--manifest",
    "--pin-file",
];
const CURRENT_REQUIRED_ARGS: &[&str] = &[
    "--apply",
    "--target",
    "--target-sha256",
    "--staged",
    "--staged-sha256",
    "--helper-target",
    "--helper-sha256",
    "--cli-staged",
    "--cli-target",
    "--cli-sha256",
    "--archive",
    "--parent-pid",
    "--version",
    "--last-applied",
    "--error-file",
    "--manifest",
    "--pin-file",
];
const LEGACY_VALUE_ARGS: &[&str] = &[
    "--target",
    "--staged",
    "--staged-sha256",
    "--helper-sha256",
    "--parent-pid",
    "--version",
    "--last-applied",
    "--error-file",
];
const LEGACY_REQUIRED_ARGS: &[&str] = &[
    "--apply",
    "--target",
    "--staged",
    "--staged-sha256",
    "--parent-pid",
    "--version",
    "--last-applied",
    "--error-file",
];

#[derive(Debug)]
pub(crate) enum ApplyRequest {
    Current(Box<ApplyArgs>),
    Legacy(Box<LegacyApplyArgs>),
}

impl ApplyRequest {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, String> {
        if CURRENT_PROTOCOL_ARGS.iter().any(|key| has_key(raw, key)) {
            ApplyArgs::parse(raw).map(Box::new).map(Self::Current)
        } else {
            LegacyApplyArgs::parse(raw).map(Box::new).map(Self::Legacy)
        }
    }

    pub(crate) fn error_file(&self) -> &std::path::Path {
        match self {
            Self::Current(args) => &args.error_file,
            Self::Legacy(args) => &args.error_file,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ApplyArgs {
    pub(crate) target: PathBuf,
    pub(crate) target_sha256: String,
    pub(crate) staged: PathBuf,
    pub(crate) staged_sha256: String,
    pub(crate) helper_target: PathBuf,
    pub(crate) helper_sha256: String,
    pub(crate) cli_staged: PathBuf,
    pub(crate) cli_target: PathBuf,
    pub(crate) cli_sha256: String,
    pub(crate) archive: PathBuf,
    pub(crate) parent_pid: u32,
    pub(crate) version: String,
    pub(crate) last_applied: PathBuf,
    pub(crate) error_file: PathBuf,
    pub(crate) manifest: PathBuf,
    pub(crate) pin_file: PathBuf,
}

/// Arguments emitted by Smart Explorer 0.5.119. That caller had
/// already installed the feed's CLI and helper before launching the helper,
/// so the compatibility worker replaces only the app. The staged app hash is
/// mandatory here even though the historical parser treated it as optional.
#[derive(Debug)]
pub(crate) struct LegacyApplyArgs {
    pub(crate) target: PathBuf,
    pub(crate) staged: PathBuf,
    pub(crate) staged_sha256: String,
    pub(crate) helper_sha256: Option<String>,
    pub(crate) parent_pid: u32,
    pub(crate) version: String,
    pub(crate) last_applied: PathBuf,
    pub(crate) error_file: PathBuf,
}

impl ApplyArgs {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, String> {
        validate_shape(raw, CURRENT_VALUE_ARGS, CURRENT_REQUIRED_ARGS, "Update")?;
        if has_key(raw, "--elevated") {
            return Err(
                "Update darf nicht aus einem erhoehten Helfer in die GUI weitergereicht werden; Installer verwenden"
                    .to_string(),
            );
        }
        Ok(Self {
            target: PathBuf::from(required_arg(raw, "--target")?),
            target_sha256: required_sha256(raw, "--target-sha256")?,
            staged: PathBuf::from(required_arg(raw, "--staged")?),
            staged_sha256: required_sha256(raw, "--staged-sha256")?,
            helper_target: PathBuf::from(required_arg(raw, "--helper-target")?),
            helper_sha256: required_sha256(raw, "--helper-sha256")?,
            cli_staged: PathBuf::from(required_arg(raw, "--cli-staged")?),
            cli_target: PathBuf::from(required_arg(raw, "--cli-target")?),
            cli_sha256: required_sha256(raw, "--cli-sha256")?,
            archive: PathBuf::from(required_arg(raw, "--archive")?),
            parent_pid: required_arg(raw, "--parent-pid")?
                .parse()
                .map_err(|e| format!("parent pid ungueltig: {}", e))?,
            version: required_arg(raw, "--version")?,
            last_applied: PathBuf::from(required_arg(raw, "--last-applied")?),
            error_file: PathBuf::from(required_arg(raw, "--error-file")?),
            manifest: PathBuf::from(required_arg(raw, "--manifest")?),
            pin_file: PathBuf::from(required_arg(raw, "--pin-file")?),
        })
    }
}

impl LegacyApplyArgs {
    fn parse(raw: &[String]) -> Result<Self, String> {
        validate_shape(raw, LEGACY_VALUE_ARGS, LEGACY_REQUIRED_ARGS, "Legacy")?;
        let helper_sha256 = optional_sha256(raw, "--helper-sha256")?;
        if has_key(raw, "--elevated") {
            return Err(
                "Legacy-Update darf nicht ueber einen nicht atomaren UAC-Handoff fortgesetzt werden; Installer verwenden"
                    .to_string(),
            );
        }
        Ok(Self {
            target: PathBuf::from(required_arg(raw, "--target")?),
            staged: PathBuf::from(required_arg(raw, "--staged")?),
            staged_sha256: required_sha256(raw, "--staged-sha256")?,
            helper_sha256,
            parent_pid: required_arg(raw, "--parent-pid")?
                .parse()
                .map_err(|e| format!("parent pid ungueltig: {}", e))?,
            version: required_arg(raw, "--version")?,
            last_applied: PathBuf::from(required_arg(raw, "--last-applied")?),
            error_file: PathBuf::from(required_arg(raw, "--error-file")?),
        })
    }
}

fn validate_shape(
    raw: &[String],
    value_args: &[&str],
    required_args: &[&str],
    protocol: &str,
) -> Result<(), String> {
    let mut seen = Vec::new();
    let mut index = 1usize;
    while index < raw.len() {
        let key = raw[index].as_str();
        if seen.contains(&key) {
            return Err(format!("{protocol}-Argument {key} ist doppelt"));
        }
        seen.push(key);
        match key {
            "--apply" | "--elevated" => index += 1,
            key if value_args.contains(&key) => {
                if index + 1 >= raw.len() {
                    return Err(format!("Argument {key} fehlt"));
                }
                index += 2;
            }
            _ => return Err(format!("Unbekanntes {protocol}-Argument {key}")),
        }
    }
    for key in required_args {
        if !seen.contains(key) {
            return Err(format!("Argument {key} fehlt"));
        }
    }
    Ok(())
}

fn required_arg(raw: &[String], key: &str) -> Result<String, String> {
    arg_value(raw, key).ok_or_else(|| format!("Argument {} fehlt", key))
}

fn required_sha256(raw: &[String], key: &str) -> Result<String, String> {
    normalize_sha256(&required_arg(raw, key)?).map_err(|error| format!("Argument {key}: {error}"))
}

fn optional_sha256(raw: &[String], key: &str) -> Result<Option<String>, String> {
    arg_value(raw, key)
        .map(|value| normalize_sha256(&value).map_err(|error| format!("Argument {key}: {error}")))
        .transpose()
}

pub(crate) fn arg_value(raw: &[String], key: &str) -> Option<String> {
    let mut index = 1usize;
    while index < raw.len() {
        let candidate = raw[index].as_str();
        if candidate == key {
            return raw.get(index + 1).cloned();
        }
        index += if matches!(candidate, "--apply" | "--elevated") {
            1
        } else {
            2
        };
    }
    None
}

pub(crate) fn has_key(raw: &[String], key: &str) -> bool {
    let mut index = 1usize;
    while index < raw.len() {
        let candidate = raw[index].as_str();
        if candidate == key {
            return true;
        }
        index += if matches!(candidate, "--apply" | "--elevated") {
            1
        } else {
            2
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> Vec<String> {
        vec![
            "smart_explorer_updater".into(),
            "--apply".into(),
            "--target".into(),
            "target.exe".into(),
            "--target-sha256".into(),
            "a".repeat(64),
            "--staged".into(),
            "staged.exe".into(),
            "--staged-sha256".into(),
            "b".repeat(64),
            "--helper-target".into(),
            "helper-target.exe".into(),
            "--helper-sha256".into(),
            "c".repeat(64),
            "--cli-staged".into(),
            "cli-staged.exe".into(),
            "--cli-target".into(),
            "cli-target.exe".into(),
            "--cli-sha256".into(),
            "d".repeat(64),
            "--archive".into(),
            "archive.exe".into(),
            "--parent-pid".into(),
            "42".into(),
            "--version".into(),
            "1.2.3".into(),
            "--last-applied".into(),
            "last.txt".into(),
            "--error-file".into(),
            "error.txt".into(),
            "--manifest".into(),
            "manifest.json".into(),
            "--pin-file".into(),
            "pin.txt".into(),
        ]
    }

    #[test]
    fn parse_requires_and_normalizes_all_sha256_args() {
        let args = ApplyArgs::parse(&base_args()).unwrap();
        assert_eq!(args.target_sha256, "a".repeat(64));
        assert_eq!(args.staged_sha256, "b".repeat(64));
        assert_eq!(args.helper_sha256, "c".repeat(64));
        assert_eq!(args.cli_sha256, "d".repeat(64));
    }

    #[test]
    fn parse_rejects_invalid_sha256_args() {
        let mut raw = base_args();
        let index = raw.iter().position(|arg| arg == "--staged-sha256").unwrap() + 1;
        raw[index] = "nope".into();

        assert!(ApplyArgs::parse(&raw).is_err());
    }

    #[test]
    fn parse_rejects_missing_mandatory_hash() {
        let mut raw = base_args();
        let index = raw.iter().position(|arg| arg == "--helper-sha256").unwrap();
        raw.drain(index..=index + 1);
        assert!(ApplyArgs::parse(&raw).is_err());
    }

    #[test]
    fn current_protocol_rejects_unknown_or_duplicate_arguments() {
        let mut unknown = base_args();
        unknown.extend(["--surprise".into(), "value".into()]);
        assert!(ApplyRequest::parse(&unknown).is_err());

        let mut duplicate = base_args();
        duplicate.extend(["--version".into(), "9.9.9".into()]);
        assert!(ApplyRequest::parse(&duplicate).is_err());
    }

    #[test]
    fn current_protocol_rejects_elevated_gui_handoff() {
        let mut raw = base_args();
        raw.push("--elevated".into());
        assert!(ApplyArgs::parse(&raw).is_err());
    }

    fn legacy_args() -> Vec<String> {
        vec![
            "smart_explorer_updater".into(),
            "--apply".into(),
            "--target".into(),
            "target.exe".into(),
            "--staged".into(),
            "staged.exe".into(),
            "--parent-pid".into(),
            "42".into(),
            "--version".into(),
            "0.5.121".into(),
            "--last-applied".into(),
            "last.txt".into(),
            "--error-file".into(),
            "error.txt".into(),
            "--staged-sha256".into(),
            "b".repeat(64),
        ]
    }

    #[test]
    fn request_accepts_exact_legacy_0_5_119_protocol() {
        let request = ApplyRequest::parse(&legacy_args()).unwrap();
        let ApplyRequest::Legacy(args) = request else {
            panic!("legacy request was misclassified")
        };
        assert_eq!(args.staged_sha256, "b".repeat(64));
        assert!(args.helper_sha256.is_none());
    }

    #[test]
    fn legacy_protocol_requires_staged_hash() {
        let mut raw = legacy_args();
        let index = raw.iter().position(|arg| arg == "--staged-sha256").unwrap();
        raw.drain(index..=index + 1);
        assert!(ApplyRequest::parse(&raw).is_err());
    }

    #[test]
    fn partial_current_protocol_never_downgrades_to_legacy() {
        let mut raw = legacy_args();
        raw.extend(["--helper-target".into(), "helper.exe".into()]);
        assert!(ApplyRequest::parse(&raw).is_err());
    }

    #[test]
    fn legacy_protocol_rejects_any_elevation_handoff() {
        let mut raw = legacy_args();
        raw.extend([
            "--helper-sha256".into(),
            "c".repeat(64),
            "--elevated".into(),
        ]);
        assert!(ApplyRequest::parse(&raw).is_err());
    }

    #[test]
    fn protocol_detection_ignores_flag_like_values() {
        let mut raw = legacy_args();
        let version = raw.iter().position(|arg| arg == "--version").unwrap() + 1;
        raw[version] = "--manifest".into();

        let request = ApplyRequest::parse(&raw).unwrap();
        let ApplyRequest::Legacy(args) = request else {
            panic!("flag-like value changed protocol classification")
        };
        assert_eq!(args.version, "--manifest");
    }

    #[test]
    fn legacy_protocol_rejects_unknown_or_duplicate_arguments() {
        let mut unknown = legacy_args();
        unknown.extend(["--surprise".into(), "value".into()]);
        assert!(ApplyRequest::parse(&unknown).is_err());

        let mut duplicate = legacy_args();
        duplicate.extend(["--version".into(), "0.5.122".into()]);
        assert!(ApplyRequest::parse(&duplicate).is_err());
    }
}
