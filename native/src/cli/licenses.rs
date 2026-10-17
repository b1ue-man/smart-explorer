//! `se licenses [--json]` (plan C4): the license texts of every Rust crate
//! embedded in this program. The text form prints the file unchanged;
//! `--json` prints `{"embedded", "sha256", "crates": [{name, version,
//! license}]}`. A development build without the embedded file prints the
//! reason (JSON: `embedded: false` plus `reason`) and exits 1. The MCP tool
//! `se` reaches the same command.

use std::io::{self, Write};

use serde_json::Value;

use crate::licenses;

/// Runs `se licenses` on stdout/stderr and returns the process exit code:
/// 0 with an embedded file, 1 without one or when writing fails.
pub(crate) fn run(json: bool) -> i32 {
    let stdout = io::stdout();
    let stderr = io::stderr();
    run_to(json, &mut stdout.lock(), &mut stderr.lock())
}

/// [`run`] with explicit writers.
pub(crate) fn run_to(json: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let embedded = licenses::text();
    let written = if json {
        write_json(out, embedded.is_some())
    } else {
        match embedded {
            Some(text) => out.write_all(text.as_bytes()).and_then(|()| out.flush()),
            None => writeln!(err, "{}", licenses::MISSING_NOTICE),
        }
    };
    match written {
        // A closed pipe (`se licenses | head`) is not an error of the command.
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            let _ = writeln!(err, "se licenses: Ausgabe fehlgeschlagen: {error}");
            1
        }
        _ if embedded.is_none() => 1,
        _ => 0,
    }
}

fn write_json(out: &mut dyn Write, embedded: bool) -> io::Result<()> {
    let mut value = licenses::summary_json();
    if !embedded {
        value["reason"] = Value::from(licenses::MISSING_NOTICE);
    }
    let text = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
    writeln!(out, "{text}")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::run_to;
    use crate::licenses;

    fn run_captured(json: bool) -> (i32, Vec<u8>, Vec<u8>) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_to(json, &mut out, &mut err);
        (code, out, err)
    }

    fn keys(value: &Value) -> BTreeSet<&str> {
        value
            .as_object()
            .expect("JSON object")
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn licenses_task_cli_json_shape() {
        let (code, out, err) = run_captured(true);
        let value: Value = serde_json::from_slice(&out).expect("se licenses --json prints JSON");
        match licenses::text() {
            Some(text) => {
                assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
                assert_eq!(
                    keys(&value),
                    BTreeSet::from(["crates", "embedded", "sha256"])
                );
                assert_eq!(value["embedded"], Value::Bool(true));
                let sha256 = value["sha256"].as_str().expect("sha256 is a string");
                assert_eq!(sha256.len(), 64, "sha256 has 64 hex digits: {sha256}");
                assert!(
                    sha256
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                    "sha256 is lowercase hex: {sha256}"
                );
                assert_eq!(sha256, licenses::lf_sha256(text));
                let crates = value["crates"].as_array().expect("crates is an array");
                assert!(!crates.is_empty(), "crates lists the embedded crates");
                assert_eq!(crates.len(), licenses::crates().len());
                for entry in crates {
                    assert_eq!(keys(entry), BTreeSet::from(["license", "name", "version"]));
                    for field in ["license", "name", "version"] {
                        let field_text = entry[field].as_str().unwrap_or_default();
                        assert!(!field_text.trim().is_empty(), "{field} is empty in {entry}");
                    }
                }

                let (code, out, err) = run_captured(false);
                assert_eq!(code, 0);
                assert!(
                    out == text.as_bytes(),
                    "se licenses prints the embedded file unchanged"
                );
                assert!(err.is_empty(), "stderr: {}", String::from_utf8_lossy(&err));
            }
            None => {
                assert_eq!(code, 1, "without an embedded file se licenses exits 1");
                assert_eq!(
                    keys(&value),
                    BTreeSet::from(["crates", "embedded", "reason", "sha256"])
                );
                assert_eq!(value["embedded"], Value::Bool(false));
                assert_eq!(value["sha256"], Value::Null);
                assert_eq!(value["crates"], Value::Array(Vec::new()));
                assert_eq!(value["reason"], Value::from(licenses::MISSING_NOTICE));

                let (code, out, err) = run_captured(false);
                assert_eq!(code, 1);
                assert!(out.is_empty());
                assert!(String::from_utf8_lossy(&err).contains(licenses::MISSING_NOTICE));
            }
        }
    }
}
