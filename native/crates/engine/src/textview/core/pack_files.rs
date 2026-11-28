//! Files of language packs: finding the `.sublime-syntax` files of a pack
//! (one file, or a Sublime package folder searched recursively without
//! following links), reading and validating them, and pack names.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use syntect::parsing::SyntaxDefinition;

use super::error::{TextViewError, TextViewResult};
use super::packs::PackSource;

/// Extension of a Sublime syntax definition.
pub const SYNTAX_EXTENSION: &str = "sublime-syntax";
/// Larger syntax files are refused (the largest bat ships has about 300 KiB).
const MAX_SYNTAX_BYTES: u64 = 16 * 1024 * 1024;
/// Deepest folder level searched inside a pack.
const MAX_DEPTH: usize = 8;

/// One syntax file of a pack: path relative to the pack (with `/`), bytes.
#[derive(Clone, Debug)]
pub(crate) struct SyntaxFile {
    pub(crate) rel: String,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn is_syntax_file_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(SYNTAX_EXTENSION))
}

/// License files a pack folder carries at its top level.
pub(crate) fn is_license_file_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["LICENSE", "LICENCE", "COPYING", "NOTICE"]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

/// Checks a pack name: 1–100 characters, a plain file name on every system.
pub(crate) fn check_pack_name(name: &str) -> TextViewResult<()> {
    let bad = name.is_empty()
        || name.chars().count() > 100
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if bad {
        Err(TextViewError::InvalidPackName(name.to_string()))
    } else {
        Ok(())
    }
}

/// The `.sublime-syntax` files below `dir`, sorted by relative path. Links
/// are not followed; deeper than [`MAX_DEPTH`] levels is not searched.
pub(crate) fn syntax_files_in(dir: &Path) -> io::Result<Vec<SyntaxFile>> {
    let mut found = Vec::new();
    collect(dir, "", 0, &mut found)?;
    found.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(found)
}

fn collect(dir: &Path, prefix: &str, depth: usize, found: &mut Vec<SyntaxFile>) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if kind.is_dir() {
            collect(&entry.path(), &rel, depth + 1, found)?;
        } else if kind.is_file() && is_syntax_file_name(&name) {
            let bytes = read_capped(&entry.path())?;
            found.push(SyntaxFile { rel, bytes });
        }
    }
    Ok(())
}

/// Reads a syntax file, refusing one over [`MAX_SYNTAX_BYTES`].
pub(crate) fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    let len = fs::metadata(path)?.len();
    if len > MAX_SYNTAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} ist größer als 16 MiB", path.display()),
        ));
    }
    fs::read(path)
}

/// Parses one syntax file; the error is a German message naming the file.
pub(crate) fn parse_syntax(file: &SyntaxFile) -> Result<SyntaxDefinition, String> {
    let text = std::str::from_utf8(&file.bytes)
        .map_err(|_| format!("{}: die Datei ist kein UTF-8-Text", file.rel))?;
    let fallback = Path::new(&file.rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Sprache");
    SyntaxDefinition::load_from_str(text, true, Some(fallback))
        .map_err(|error| format!("{}: keine gültige Sublime-Syntax ({error})", file.rel))
}

/// What `add` was handed: default name, syntax files, top-level license
/// files and whether it is a folder pack.
pub(crate) struct Gathered {
    pub(crate) name: String,
    pub(crate) files: Vec<SyntaxFile>,
    pub(crate) licenses: Vec<PathBuf>,
    pub(crate) folder: bool,
}

pub(crate) fn gather(source: &PackSource) -> TextViewResult<Gathered> {
    match source {
        PackSource::Bytes { file_name, bytes } => {
            let stem = syntax_stem(file_name)?;
            let file = SyntaxFile {
                rel: file_name.clone(),
                bytes: bytes.clone(),
            };
            Ok(Gathered {
                name: stem,
                files: vec![file],
                licenses: Vec::new(),
                folder: false,
            })
        }
        PackSource::Path(path) if path.is_dir() => {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let files = syntax_files_in(path)?;
            if files.is_empty() {
                return Err(TextViewError::InvalidPack(format!(
                    "der Ordner {} enthält keine .sublime-syntax-Datei",
                    path.display()
                )));
            }
            let licenses = license_files(path)
                .into_iter()
                .map(|n| path.join(n))
                .collect();
            Ok(Gathered {
                name,
                files,
                licenses,
                folder: true,
            })
        }
        PackSource::Path(path) => {
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let stem = syntax_stem(&file_name)?;
            let bytes = read_capped(path)?;
            Ok(Gathered {
                name: stem,
                files: vec![SyntaxFile {
                    rel: file_name,
                    bytes,
                }],
                licenses: Vec::new(),
                folder: false,
            })
        }
    }
}

fn syntax_stem(file_name: &str) -> TextViewResult<String> {
    if !is_syntax_file_name(file_name) {
        return Err(TextViewError::InvalidPack(format!(
            "„{file_name}“ ist keine .sublime-syntax-Datei (Sublime-Syntax wie in Sublime Text und bat)"
        )));
    }
    Ok(file_name[..file_name.len() - SYNTAX_EXTENSION.len() - 1].to_string())
}

pub(crate) fn parse_all(files: &[SyntaxFile]) -> (Vec<SyntaxDefinition>, Vec<String>) {
    let mut definitions = Vec::new();
    let mut problems = Vec::new();
    for file in files {
        match parse_syntax(file) {
            Ok(definition) => definitions.push(definition),
            Err(reason) => problems.push(reason),
        }
    }
    (definitions, problems)
}

pub(crate) fn license_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| is_license_file_name(n))
        .collect();
    names.sort();
    names
}

pub(crate) fn added_license(files: &[String]) -> String {
    if files.is_empty() {
        "keine Lizenzdatei im Paket".to_string()
    } else {
        format!("Lizenzdatei im Paket: {}", files.join(", "))
    }
}
