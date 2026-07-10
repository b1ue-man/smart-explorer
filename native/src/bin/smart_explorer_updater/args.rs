use std::path::PathBuf;

use super::hash::normalize_sha256;

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
    pub(crate) elevated: bool,
}

impl ApplyArgs {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, String> {
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
            elevated: raw.iter().any(|a| a == "--elevated"),
        })
    }
}

fn required_arg(raw: &[String], key: &str) -> Result<String, String> {
    arg_value(raw, key).ok_or_else(|| format!("Argument {} fehlt", key))
}

fn required_sha256(raw: &[String], key: &str) -> Result<String, String> {
    normalize_sha256(&required_arg(raw, key)?).map_err(|error| format!("Argument {key}: {error}"))
}

pub(crate) fn arg_value(raw: &[String], key: &str) -> Option<String> {
    raw.iter()
        .position(|a| a == key)
        .and_then(|i| raw.get(i + 1))
        .cloned()
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
}
