//! Language packs: the built-in languages of two-face (each its own entry)
//! and the packs a user added to the language folder, as Sublime Text, bat
//! and syntect manage them: a pack is one `.sublime-syntax` file
//! (`<folder>/<name>.sublime-syntax`) or a Sublime package folder
//! (`<folder>/<name>/…`). Added definitions are loaded after the built-in
//! ones, so they win for the same name, extension or first line. Invalid
//! packs are refused by `add` and skipped (and reported) when loading.
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use syntect::parsing::{SyntaxDefinition, SyntaxSet};

use super::builtin_licenses::BuiltinLicenses;
use super::error::{TextViewError, TextViewResult};
use super::highlight::{Highlighter, LanguageInfo};
use super::pack_cache;
use super::pack_files::{
    added_license, check_pack_name, gather, is_syntax_file_name, license_files, parse_all,
    parse_syntax, read_capped, syntax_files_in, Gathered, SyntaxFile, SYNTAX_EXTENSION,
};

/// Where an entry of the list comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackOrigin {
    /// Built into se (two-face, the syntax set of bat).
    Builtin,
    /// Added by the user to the language folder.
    Added,
}

/// One entry of [`LanguageStore::list`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LanguagePack {
    /// Pack name (built-in: the language name).
    pub name: String,
    pub origin: PackOrigin,
    /// Languages the pack defines with their extensions.
    pub languages: Vec<LanguageInfo>,
    /// License note (built-in: license and source file in two-face; added:
    /// the license files found in the pack).
    pub license: String,
    /// File or folder of an added pack.
    pub path: Option<PathBuf>,
    /// Built-in entry: the added pack that takes over its name or at least
    /// one of its extensions.
    pub shadowed_by: Option<String>,
    /// Added pack: files skipped while loading, with the reason.
    pub problems: Vec<String>,
}

/// What `add` installs.
#[derive(Clone, Debug)]
pub enum PackSource {
    /// A `.sublime-syntax` file or a folder holding such files.
    Path(PathBuf),
    /// The bytes of one `.sublime-syntax` file (Android picker, drag & drop).
    Bytes { file_name: String, bytes: Vec<u8> },
}

/// One added pack as found on disk.
struct AddedPack {
    name: String,
    path: PathBuf,
    files: Vec<SyntaxFile>,
    licenses: Vec<String>,
}

/// The language folder and its cache folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageStore {
    folder: PathBuf,
    cache: PathBuf,
}

impl LanguageStore {
    /// A store over `folder` (packs) and `cache` (compiled syntax sets).
    pub fn new(folder: impl Into<PathBuf>, cache: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
            cache: cache.into(),
        }
    }

    /// The store in the app data folder (`<app data>/languages`).
    pub fn default_store() -> Self {
        Self::new(
            crate::textview::os::languages_folder(),
            crate::textview::os::languages_cache_folder(),
        )
    }

    /// The folder holding the added packs (may not exist yet).
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn cache_folder(&self) -> &Path {
        &self.cache
    }

    /// Added packs first (sorted by name), then every built-in language.
    pub fn list(&self) -> Vec<LanguagePack> {
        let mut out = Vec::new();
        let mut taken: Vec<(String, Vec<String>, String)> = Vec::new();
        for pack in self.added_packs() {
            let (definitions, mut problems) = parse_all(&pack.files);
            if pack.files.is_empty() {
                problems.push("keine lesbare .sublime-syntax-Datei".to_string());
            }
            let languages: Vec<LanguageInfo> = definitions
                .iter()
                .filter(|d| !d.hidden)
                .map(|d| LanguageInfo {
                    name: d.name.clone(),
                    extensions: d.file_extensions.clone(),
                })
                .collect();
            for language in &languages {
                taken.push((
                    language.name.to_lowercase(),
                    language
                        .extensions
                        .iter()
                        .map(|e| e.to_lowercase())
                        .collect(),
                    pack.name.clone(),
                ));
            }
            out.push(LanguagePack {
                name: pack.name.clone(),
                origin: PackOrigin::Added,
                languages,
                license: added_license(&pack.licenses),
                path: Some(pack.path.clone()),
                shadowed_by: None,
                problems,
            });
        }
        let licenses = BuiltinLicenses::load();
        let builtin = two_face::syntax::extra_newlines();
        let mut builtin_entries: Vec<LanguagePack> = builtin
            .syntaxes()
            .iter()
            .filter(|s| !s.hidden)
            .map(|syntax| {
                let lower = syntax.name.to_lowercase();
                let shadowed_by = taken
                    .iter()
                    .rev()
                    .find(|(name, extensions, _)| {
                        *name == lower
                            || syntax
                                .file_extensions
                                .iter()
                                .any(|e| extensions.contains(&e.to_lowercase()))
                    })
                    .map(|(_, _, pack)| pack.clone());
                LanguagePack {
                    name: syntax.name.clone(),
                    origin: PackOrigin::Builtin,
                    languages: vec![LanguageInfo {
                        name: syntax.name.clone(),
                        extensions: syntax.file_extensions.clone(),
                    }],
                    license: licenses.note(&syntax.name),
                    path: None,
                    shadowed_by,
                    problems: Vec::new(),
                }
            })
            .collect();
        builtin_entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out.extend(builtin_entries);
        out
    }

    /// Validates and installs a pack under `name` (default: the file name
    /// without `.sublime-syntax`, or the folder name). Every syntax file must
    /// parse, and the pack must not refer to contexts no loaded language has.
    pub fn add(&self, source: PackSource, name: Option<&str>) -> TextViewResult<LanguagePack> {
        let Gathered {
            name: default_name,
            files,
            licenses,
            folder: folder_pack,
        } = gather(&source)?;
        let name = name.map(str::trim).unwrap_or(&default_name).to_string();
        check_pack_name(&name)?;
        if self.pack_path(&name).is_some() {
            return Err(TextViewError::PackExists(name));
        }
        let (definitions, problems) = parse_all(&files);
        if let Some(problem) = problems.first() {
            return Err(TextViewError::InvalidPack(problem.clone()));
        }
        fs::create_dir_all(&self.folder)?;
        if folder_pack {
            pack_cache::install_folder(&self.folder, &name, &files, &licenses)?;
        } else {
            let bytes = files
                .first()
                .map(|f| f.bytes.as_slice())
                .unwrap_or_default();
            let target = format!("{name}.{SYNTAX_EXTENSION}");
            pack_cache::install_file(&self.folder, &target, bytes)?;
        }
        // Build the set the highlighter will use once, check it, keep it.
        let loaded = self.loaded_definitions();
        let files = loaded.files;
        let set = build_set(loaded.definitions);
        if let Err(error) = check_links(&set, &definitions) {
            let _ = self.remove(&name);
            return Err(error);
        }
        pack_cache::store(&self.cache, &pack_cache::cache_key(&files), &set);
        self.list()
            .into_iter()
            .find(|p| p.origin == PackOrigin::Added && p.name == name)
            .ok_or(TextViewError::PackNotFound(name))
    }

    /// Removes an added pack; a built-in language cannot be removed. The
    /// name is checked like a new one, so it never reaches outside the folder.
    pub fn remove(&self, name: &str) -> TextViewResult<()> {
        let name = name.trim();
        check_pack_name(name)?;
        match self.pack_path(name) {
            Some(path) if path.is_dir() && !path.is_symlink() => fs::remove_dir_all(&path)?,
            Some(path) => fs::remove_file(&path)?,
            None => {
                let builtin = two_face::syntax::extra_newlines();
                if builtin
                    .syntaxes()
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(name))
                {
                    return Err(TextViewError::BuiltinPack(name.to_string()));
                }
                return Err(TextViewError::PackNotFound(name.to_string()));
            }
        }
        Ok(())
    }

    /// The highlighter over the built-in languages plus every valid added
    /// definition; skipped files are in [`Highlighter::problems`]. A built
    /// set is kept in the cache folder, keyed by the SHA-256 of the files.
    pub fn highlighter(&self) -> Highlighter {
        let Loaded {
            definitions,
            files,
            problems,
        } = self.loaded_definitions();
        if definitions.is_empty() {
            return Highlighter::with_syntaxes(two_face::syntax::extra_newlines(), problems);
        }
        let key = pack_cache::cache_key(&files);
        let set = pack_cache::load(&self.cache, &key).unwrap_or_else(|| {
            let set = build_set(definitions);
            pack_cache::store(&self.cache, &key, &set);
            set
        });
        Highlighter::with_syntaxes(set, problems)
    }

    /// Cheap description of the folder's syntax files (names, sizes, times);
    /// it changes whenever a pack is added, removed or edited.
    pub fn fingerprint(&self) -> String {
        pack_cache::fingerprint(&self.folder)
    }

    /// Path of the added pack `name` (file or folder), if present.
    fn pack_path(&self, name: &str) -> Option<PathBuf> {
        let file = self.folder.join(format!("{name}.{SYNTAX_EXTENSION}"));
        let dir = self.folder.join(name);
        if file.symlink_metadata().is_ok() {
            Some(file)
        } else if dir.symlink_metadata().is_ok() {
            Some(dir)
        } else {
            None
        }
    }

    fn added_packs(&self) -> Vec<AddedPack> {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut packs = Vec::new();
        for entry in entries.flatten() {
            let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if file_name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                let files = syntax_files_in(&path).unwrap_or_default();
                packs.push(AddedPack {
                    licenses: license_files(&path),
                    name: file_name,
                    path,
                    files,
                });
            } else if kind.is_file() && is_syntax_file_name(&file_name) {
                let name = file_name[..file_name.len() - SYNTAX_EXTENSION.len() - 1].to_string();
                let files = read_capped(&path)
                    .map(|bytes| {
                        vec![SyntaxFile {
                            rel: file_name.clone(),
                            bytes,
                        }]
                    })
                    .unwrap_or_default();
                packs.push(AddedPack {
                    name,
                    path,
                    files,
                    licenses: Vec::new(),
                });
            }
        }
        packs.sort_by(|a, b| a.name.cmp(&b.name));
        packs
    }

    /// Valid definitions of all added packs in load order, the files they
    /// came from (for the cache key) and the messages about skipped files.
    fn loaded_definitions(&self) -> Loaded {
        let mut definitions = Vec::new();
        let mut used = Vec::new();
        let mut problems = Vec::new();
        for pack in self.added_packs() {
            if pack.files.is_empty() {
                problems.push(format!(
                    "Sprachpaket „{}“ übersprungen: keine lesbare .sublime-syntax-Datei",
                    pack.name
                ));
            }
            for file in pack.files {
                match parse_syntax(&file) {
                    Ok(definition) => {
                        definitions.push(definition);
                        used.push(SyntaxFile {
                            rel: format!("{}/{}", pack.name, file.rel),
                            ..file
                        });
                    }
                    Err(reason) => problems.push(format!(
                        "Sprachpaket „{}“: Datei übersprungen: {reason}",
                        pack.name
                    )),
                }
            }
        }
        Loaded {
            definitions,
            files: used,
            problems,
        }
    }
}

/// Valid added definitions in load order, their files and skip messages.
struct Loaded {
    definitions: Vec<SyntaxDefinition>,
    files: Vec<SyntaxFile>,
    problems: Vec<String>,
}

/// Built-in set plus `definitions` (added last, so they win).
fn build_set(definitions: Vec<SyntaxDefinition>) -> SyntaxSet {
    let mut builder = two_face::syntax::extra_newlines().into_builder();
    for definition in definitions {
        builder.add(definition);
    }
    builder.build()
}

/// Refuses a pack that refers to contexts (`scope:source.x`, `Packages/…`)
/// that neither the built-in languages, the added packs nor itself define.
fn check_links(set: &SyntaxSet, new: &[SyntaxDefinition]) -> TextViewResult<()> {
    let prefixes: Vec<String> = new
        .iter()
        .map(|d| format!("Syntax '{}' with scope '{}'", d.name, d.scope))
        .collect();
    let missing: Vec<String> = set
        .find_unlinked_contexts()
        .into_iter()
        .filter(|c| prefixes.iter().any(|p| c.starts_with(p.as_str())))
        .take(5)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(TextViewError::InvalidPack(format!(
            "das Paket verweist auf Kontexte, die keine geladene Sprache definiert: {}",
            missing.join("; ")
        )))
    }
}
