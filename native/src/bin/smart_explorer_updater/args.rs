use std::path::PathBuf;

use super::hash::normalize_sha256;

#[derive(Debug)]
pub(crate) struct ApplyArgs {
    pub(crate) target: PathBuf,
    pub(crate) staged: PathBuf,
    pub(crate) staged_sha256: Option<String>,
    pub(crate) helper_sha256: Option<String>,
    pub(crate) parent_pid: u32,
    pub(crate) version: String,
    pub(crate) last_applied: PathBuf,
    pub(crate) error_file: PathBuf,
    pub(crate) elevated: bool,
}

impl ApplyArgs {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, String> {
        Ok(Self {
            target: PathBuf::from(required_arg(raw, "--target")?),
            staged: PathBuf::from(required_arg(raw, "--staged")?),
            staged_sha256: optional_sha256(raw, "--staged-sha256")?,
            helper_sha256: optional_sha256(raw, "--helper-sha256")?,
            parent_pid: required_arg(raw, "--parent-pid")?
                .parse()
                .map_err(|e| format!("parent pid ungueltig: {}", e))?,
            version: required_arg(raw, "--version")?,
            last_applied: PathBuf::from(required_arg(raw, "--last-applied")?),
            error_file: PathBuf::from(required_arg(raw, "--error-file")?),
            elevated: raw.iter().any(|a| a == "--elevated"),
        })
    }
}

fn required_arg(raw: &[String], key: &str) -> Result<String, String> {
    arg_value(raw, key).ok_or_else(|| format!("Argument {} fehlt", key))
}

fn optional_sha256(raw: &[String], key: &str) -> Result<Option<String>, String> {
    arg_value(raw, key)
        .map(|value| normalize_sha256(&value).map_err(|e| format!("Argument {key}: {e}")))
        .transpose()
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
            "--staged".into(),
            "staged.exe".into(),
            "--parent-pid".into(),
            "42".into(),
            "--version".into(),
            "1.2.3".into(),
            "--last-applied".into(),
            "last.txt".into(),
            "--error-file".into(),
            "error.txt".into(),
        ]
    }

    #[test]
    fn parse_accepts_optional_sha256_args() {
        let hash = "A".repeat(64);
        let mut raw = base_args();
        raw.extend([
            "--staged-sha256".into(),
            hash.clone(),
            "--helper-sha256".into(),
            hash.clone(),
        ]);

        let args = ApplyArgs::parse(&raw).unwrap();
        let expected = hash.to_ascii_lowercase();

        assert_eq!(args.staged_sha256.as_deref(), Some(expected.as_str()));
        assert_eq!(args.helper_sha256.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn parse_rejects_invalid_sha256_args() {
        let mut raw = base_args();
        raw.extend(["--staged-sha256".into(), "nope".into()]);

        assert!(ApplyArgs::parse(&raw).is_err());
    }
}
