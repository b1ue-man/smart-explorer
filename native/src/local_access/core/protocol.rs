//! Exact-purpose admission and bounded read-handle requests; no command API.
use serde::{Deserialize, Serialize};
use std::ffi::OsString;

pub(super) const HELPER_FLAG: &str = "--local-read-helper";
pub(super) const PIPE_PREFIX: &str = r"\\.\pipe\smart-explorer-read-";
pub(super) const MAX_FRAME: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) enum ReadKind {
    Metadata,
    Directory,
    File,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadRequest {
    pub path: String,
    pub kind: ReadKind,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadReply {
    pub handle: u64,
    pub error: Option<i32>,
}

pub(super) struct Startup {
    pub pipe: String,
    pub parent: u32,
    pub image_sha256: String,
    pub root: String,
}

pub(super) fn validate_root(root: &str) -> Result<(), String> {
    let normalized = root.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1..3] != *b":/"
        || normalized[2..].contains("//")
        || normalized[3..]
            .chars()
            .any(|c| c < ' ' || "<>:\"|?*".contains(c))
        || normalized[3..]
            .split('/')
            .any(|part| part == "." || part == "..")
    {
        return Err("Lesezugriff benötigt einen absoluten lokalen Laufwerkspfad".into());
    }
    Ok(())
}

pub(super) fn contains(root: &str, path: &str) -> bool {
    if validate_root(path).is_err() {
        return false;
    }
    let root = root.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let path = path.replace('\\', "/").trim_end_matches('/').to_lowercase();
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn parse(args: &[OsString]) -> Option<Result<Startup, String>> {
    if !args.iter().any(|arg| {
        let value = arg.to_string_lossy();
        value.starts_with(HELPER_FLAG) || value.starts_with("--storage-analysis-admin")
    }) {
        return None;
    }
    Some((|| {
        if args.len() != 5 || args[0] != HELPER_FLAG {
            return Err("Ungültiger Lesehelfer-Aufruf".into());
        }
        let text: Vec<_> = args
            .iter()
            .map(|arg| {
                arg.to_str()
                    .ok_or_else(|| "Ungültiges Unicode-Argument".to_string())
            })
            .collect::<Result<_, _>>()?;
        let pipe = text[1];
        let suffix = pipe
            .strip_prefix(PIPE_PREFIX)
            .ok_or("Ungültiger Pipe-Name")?;
        if suffix.len() != 32 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Ungültiger Pipe-Name".into());
        }
        let parent = text[2]
            .parse::<u32>()
            .map_err(|_| "Ungültiger Elternprozess")?;
        if parent == 0 {
            return Err("Ungültiger Elternprozess".into());
        }
        let hash = text[3];
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Ungültige Programm-Prüfsumme".into());
        }
        validate_root(text[4])?;
        Ok(Startup {
            pipe: pipe.into(),
            parent,
            image_sha256: hash.to_ascii_lowercase(),
            root: text[4].into(),
        })
    })())
}

/// CRT quoting, including trailing slashes before the closing quote.
pub(super) fn quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        quoted.push_str(&"\\".repeat(if character == '"' {
            slashes * 2 + 1
        } else {
            slashes
        }));
        slashes = 0;
        quoted.push(character);
    }
    quoted.push_str(&"\\".repeat(slashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
