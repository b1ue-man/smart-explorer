//! License note of every built-in language. two-face embeds the licenses
//! that require acknowledgement (`two_face::acknowledgement::listing`), each
//! keyed by the folder of its syntax package in the bat repository
//! (`syntaxes/02_Extra/TOML/LICENSE`). A language is matched to a package
//! folder by its normalized name; a language without a match comes from a
//! package whose license needs no acknowledgement (Sublime Text's default
//! packages and similar), and its note points at the complete listing.
use two_face::acknowledgement::{License, LicenseType};

/// Words in package folder names that are no part of the language name.
const FOLDER_NOISE: &[&str] = &["sublime", "syntax", "highlight", "vscode", "packages"];

/// License notes of the built-in languages.
pub(crate) struct BuiltinLicenses {
    entries: Vec<(String, Vec<String>, String)>,
}

impl BuiltinLicenses {
    pub(crate) fn load() -> Self {
        let listing = two_face::acknowledgement::listing();
        let entries = listing
            .for_syntaxes()
            .iter()
            .filter_map(|license| {
                let folder = package_folder(license)?;
                let tokens = if folder.contains('(') {
                    Vec::new()
                } else {
                    folder
                        .split(['_', '-', ' '])
                        .map(normalize)
                        .filter(|t| !t.is_empty() && !FOLDER_NOISE.contains(&t.as_str()))
                        .collect()
                };
                let note = format!(
                    "{} ({})",
                    license_name(license.ty),
                    license.rel_path.to_string_lossy().replace('\\', "/")
                );
                Some((normalize(&folder), tokens, note))
            })
            .collect();
        Self { entries }
    }

    /// License note of the built-in language `name`.
    pub(crate) fn note(&self, name: &str) -> String {
        let wanted = normalize(name);
        let found = self.entries.iter().find(|(folder, tokens, _)| {
            *folder == wanted
                || (folder.len() >= 4 && wanted.starts_with(folder.as_str()))
                || folder.strip_prefix(wanted.as_str()).is_some_and(|rest| {
                    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
                })
                || tokens.iter().any(|token| {
                    *token == wanted || (token.len() >= 4 && wanted.starts_with(token.as_str()))
                })
        });
        match found {
            Some((_, _, note)) => note.clone(),
            None => format!(
                "Sublime-Text-Standardpaket oder Lizenz ohne Nennungspflicht; vollständige Liste: {}",
                two_face::acknowledgement::url()
            ),
        }
    }
}

/// `TOML` for `syntaxes/02_Extra/TOML/LICENSE`.
fn package_folder(license: &License) -> Option<String> {
    let parts: Vec<String> = license
        .rel_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (parts.len() >= 4 && parts[0] == "syntaxes").then(|| parts[2].clone())
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn license_name(ty: LicenseType) -> &'static str {
    match ty {
        LicenseType::Sublime => "Sublime-Lizenz",
        LicenseType::Mit => "MIT",
        LicenseType::Bsd2Clause => "BSD-2-Clause",
        LicenseType::Bsd2ClauseFreeBsd => "BSD-2-Clause-FreeBSD",
        LicenseType::Unlicense => "Unlicense",
        LicenseType::Bsd3Clause => "BSD-3-Clause",
        LicenseType::Apache2 => "Apache-2.0",
        LicenseType::Wtfpl => "WTFPL",
        _ => "Lizenz laut two-face",
    }
}
