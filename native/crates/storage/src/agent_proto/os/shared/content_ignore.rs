//! Ignore files of a search, as ripgrep reads them (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C "Coding-Lücken"):
//! `.ignore` in every folder, `.gitignore` in every folder of a Git work
//! tree (a folder with a `.git` entry, or below one), and `.git/info/exclude`
//! of the work tree's root. The patterns are git's (`ignore::gitignore`,
//! the crate ripgrep is built on); the precedence is ripgrep's
//! (`ignore` 0.4 `dir.rs`, `Ignore::matched_ignore`): within one kind of file
//! the deepest folder that has a verdict decides, `.ignore` before
//! `.gitignore` before `exclude`, and the Git files of folders above the
//! nearest work tree root do not apply. A path is left out only for an
//! ignore verdict; a whitelist (`!pattern`) keeps it.
//!
//! The rules come from the walk itself (`IgnoreChain::child` with the
//! files a folder holds), so the same chain serves the local walk, the walk
//! over any backend and the Share host's handle-bound walk. Folders above
//! the search root are read only where the tree can read them
//! (`ContentTree::parent_ignores`; locally `local_parents`).
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

/// The ignore file of every folder (ripgrep's `.ignore`).
pub const IGNORE_FILE: &str = ".ignore";
/// Git's ignore file of every folder of a work tree.
pub const GITIGNORE_FILE: &str = ".gitignore";
/// The entry that marks a work tree root (a folder, or a file for linked
/// work trees and submodules).
pub const GIT_ENTRY: &str = ".git";
/// `.git/info/exclude`, below `GIT_ENTRY` when it is a folder.
pub const EXCLUDE_PARTS: [&str; 2] = ["info", "exclude"];
/// Longest ignore file read: git's own limit (`PATTERN_MAX_FILE_SIZE`,
/// 100 MiB, git `dir.c`); a longer one is not read.
pub const IGNORE_FILE_MAX: u64 = 100 * 1024 * 1024;

/// The ignore files one folder holds, as bytes (`None` = absent or not
/// readable), and whether it is a work tree root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IgnoreFiles {
    pub ignore: Option<Vec<u8>>,
    pub gitignore: Option<Vec<u8>>,
    pub exclude: Option<Vec<u8>>,
    pub has_git: bool,
}

impl IgnoreFiles {
    /// Nothing that changes a verdict.
    pub fn is_empty(&self) -> bool {
        self.ignore.is_none() && self.gitignore.is_none() && self.exclude.is_none() && !self.has_git
    }
}

/// The ignore files of a folder above the search root: `lead` is the path
/// from that folder down to the root (`/`-separated, never empty).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParentIgnore {
    pub lead: String,
    pub files: IgnoreFiles,
}

/// Reads at most `IGNORE_FILE_MAX` bytes of an ignore file; a longer one,
/// or one that fails, is `None` (the search then admits more, never less).
pub fn read_ignore_file(reader: &mut dyn Read) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut limited = reader.take(IGNORE_FILE_MAX + 1);
    limited.read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= IGNORE_FILE_MAX).then_some(bytes)
}

/// Where a folder of the chain lies relative to the search root.
#[derive(Debug)]
enum Base {
    /// A folder of the tree (its path below the root, "" = the root).
    Inside(String),
    /// A folder above the root; the path from it down to the root.
    Above(String),
}

#[derive(Debug)]
struct Node {
    ignore: Option<Gitignore>,
    gitignore: Option<Gitignore>,
    exclude: Option<Gitignore>,
    has_git: bool,
    /// Any folder of the chain up to here is a work tree root.
    any_git: bool,
    base: Base,
    parent: Option<Arc<Node>>,
}

impl Node {
    /// `rel` (below the search root) relative to this node's folder.
    fn relative(&self, rel: &str) -> String {
        match &self.base {
            Base::Inside(base) if base.is_empty() => rel.to_string(),
            Base::Inside(base) => rel
                .strip_prefix(base.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
                .unwrap_or(rel)
                .to_string(),
            Base::Above(lead) if rel.is_empty() => lead.clone(),
            Base::Above(lead) => format!("{lead}/{rel}"),
        }
    }
}

/// The ignore rules that apply in one folder of a walk: its own and those of
/// every folder above it. Cheap to clone; a folder without ignore files
/// shares its parent's chain.
#[derive(Clone, Debug, Default)]
pub struct IgnoreChain {
    head: Option<Arc<Node>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Ignore,
    Whitelist,
}

fn verdict(matcher: &Option<Gitignore>, path: &str, is_dir: bool) -> Option<Verdict> {
    match matcher.as_ref()?.matched(Path::new(path), is_dir) {
        Match::None => None,
        Match::Ignore(_) => Some(Verdict::Ignore),
        Match::Whitelist(_) => Some(Verdict::Whitelist),
    }
}

/// The patterns of one ignore file. Lines that are not UTF-8 or not a valid
/// pattern are left out, the others apply (ripgrep's partial errors).
fn compile(bytes: &Option<Vec<u8>>) -> Option<Gitignore> {
    let bytes = bytes.as_ref()?;
    let mut builder = GitignoreBuilder::new(".");
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        let line = line.strip_suffix('\r').unwrap_or(line);
        // Git skips a UTF-8 byte order mark at the start of the file.
        let line = if index == 0 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        let _ = builder.add_line(None, line);
    }
    builder.build().ok().filter(|matcher| !matcher.is_empty())
}

impl IgnoreChain {
    /// The chain of the folders above the search root, `parents` nearest
    /// first (as `ContentTree::parent_ignores` lists them).
    pub fn above_root(parents: &[ParentIgnore]) -> Self {
        let mut chain = Self::default();
        for parent in parents.iter().rev() {
            chain = chain.push(Base::Above(parent.lead.clone()), &parent.files);
        }
        chain
    }

    /// The chain inside the folder `folder` (below the search root) that
    /// holds `files`.
    pub fn child(&self, folder: &str, files: &IgnoreFiles) -> Self {
        self.push(Base::Inside(folder.to_string()), files)
    }

    fn push(&self, base: Base, files: &IgnoreFiles) -> Self {
        if files.is_empty() {
            return self.clone();
        }
        let any_git = files.has_git || self.head.as_ref().is_some_and(|head| head.any_git);
        Self {
            head: Some(Arc::new(Node {
                ignore: compile(&files.ignore),
                gitignore: compile(&files.gitignore),
                exclude: if files.has_git {
                    compile(&files.exclude)
                } else {
                    None
                },
                has_git: files.has_git,
                any_git,
                base,
                parent: self.head.clone(),
            })),
        }
    }

    /// No rule applies anywhere in this chain.
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// Whether `rel` (below the search root; a folder with `is_dir`) is left
    /// out by the ignore files of the folders on its way.
    pub fn ignored(&self, rel: &str, is_dir: bool) -> bool {
        let Some(head) = &self.head else {
            return false;
        };
        let any_git = head.any_git;
        let (mut by_ignore, mut by_git, mut by_exclude) = (None, None, None);
        let mut saw_git = false;
        let mut node = Some(head);
        while let Some(current) = node {
            let path = current.relative(rel);
            if by_ignore.is_none() {
                by_ignore = verdict(&current.ignore, &path, is_dir);
            }
            if any_git && !saw_git {
                if by_git.is_none() {
                    by_git = verdict(&current.gitignore, &path, is_dir);
                }
                if by_exclude.is_none() {
                    by_exclude = verdict(&current.exclude, &path, is_dir);
                }
            }
            saw_git = saw_git || current.has_git;
            node = current.parent.as_ref();
        }
        by_ignore.or(by_git).or(by_exclude) == Some(Verdict::Ignore)
    }
}

/// The ignore files of the local folder `folder`, read without leaving it
/// (`.git/info/exclude` only below a `.git` folder). Unreadable files count
/// as absent.
pub fn local_folder_files(folder: &Path) -> IgnoreFiles {
    let read = |path: &Path| -> Option<Vec<u8>> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let mut file = std::fs::File::open(path).ok()?;
        read_ignore_file(&mut file)
    };
    let git = folder.join(GIT_ENTRY);
    let git_meta = std::fs::symlink_metadata(&git).ok();
    let has_git = git_meta.is_some();
    let exclude = match git_meta {
        Some(meta) if meta.is_dir() => read(&git.join(EXCLUDE_PARTS[0]).join(EXCLUDE_PARTS[1])),
        _ => None,
    };
    IgnoreFiles {
        ignore: read(&folder.join(IGNORE_FILE)),
        gitignore: read(&folder.join(GITIGNORE_FILE)),
        exclude,
        has_git,
    }
}

/// The ignore files of the local folders above `root`, nearest first. Git
/// files count only up to the nearest work tree root (above it git and
/// ripgrep ignore them); `.ignore` files of every folder above count.
pub fn local_parents(root: &Path) -> Vec<ParentIgnore> {
    let absolute = if root.is_absolute() {
        root.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(dir) => dir.join(root),
            Err(_) => return Vec::new(),
        }
    };
    let mut parents = Vec::new();
    let mut lead: Vec<String> = Vec::new();
    let mut current = absolute.as_path();
    // Whether the root itself or a folder between it and `current` is a
    // work tree root: then no Git file above applies.
    let mut git_seen = absolute.join(GIT_ENTRY).symlink_metadata().is_ok();
    while let (Some(parent), Some(name)) = (current.parent(), current.file_name()) {
        lead.insert(0, name.to_string_lossy().into_owned());
        let mut files = local_folder_files(parent);
        if git_seen {
            files.gitignore = None;
            files.exclude = None;
            files.has_git = false;
        }
        git_seen = git_seen || files.has_git;
        if !files.is_empty() {
            parents.push(ParentIgnore {
                lead: lead.join("/"),
                files,
            });
        }
        current = parent;
    }
    parents
}

#[cfg(test)]
#[path = "content_ignore_tests.rs"]
mod tests;
