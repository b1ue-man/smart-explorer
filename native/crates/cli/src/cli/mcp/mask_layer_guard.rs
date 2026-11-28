//! Smart Explorer's own data while a mask list is active (plan 2026-11-28,
//! S1 hardening K3): the se app data (mask store, sessions, settings) and the
//! agent hosts' configuration (hooks, managed settings) are out of reach of
//! the agent's se tools, so an agent cannot weaken the lock or read the store
//! through se itself. `engine::masks::guarded_paths` names these places.
//!
//! - The file tools refuse a target inside a guarded place
//!   ([`Reach::Entry`]); `remove`, `copy` and `move` also refuse a target
//!   that contains one ([`Reach::Tree`]), because they act on everything
//!   below it.
//! - The `se` tool refuses commands that name a guarded place, `se masks`
//!   (lists are changed only by the user, at a terminal or in the desktop
//!   app; the agent uses the `masks` tool), the internal elevated helper and
//!   removing the agent plugin.
//!
//! Paths are compared after resolving links of their existing part and
//! without regard to case on every system (a variant in case is refused
//! rather than let through). Without an active list nothing is refused, as
//! before.
use super::session::Session;
use super::targets::Resolved;
use crate::masks::MaskSet;
use std::path::{Path, PathBuf};

/// What a tool does with its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reach {
    /// The target itself (list, stat, read, write, edit, mkdir, root).
    Entry,
    /// The target and everything below it (remove, copy, move).
    Tree,
}

/// [`guard`] with the session's active lists.
pub(super) fn check(session: &Session, target: &Resolved, reach: Reach) -> Result<(), String> {
    guard(&session.masks.set()?, target, reach)
}

/// Refuses `target` when it touches a guarded place while `set` is active.
pub(super) fn guard(set: &MaskSet, target: &Resolved, reach: Reach) -> Result<(), String> {
    if set.is_empty() || !target.backend.is_local() {
        return Ok(());
    }
    guard_local(&crate::masks::guarded_paths(), Path::new(&target.path), reach).map_err(
        |place| {
            format!(
                "{} {} Smart Explorer's own data or an agent host's configuration ({}); se tools leave these alone while a mask list is active",
                target.spec,
                if place.within { "is inside" } else { "contains" },
                place.path.display()
            )
        },
    )
}

/// The guarded place a path touches.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Touched {
    pub(super) path: PathBuf,
    /// The path is inside the place (else it contains the place).
    pub(super) within: bool,
}

pub(super) fn guard_local(guarded: &[PathBuf], path: &Path, reach: Reach) -> Result<(), Touched> {
    let candidate = key(&effective(path));
    for place in guarded {
        let base = key(&effective(place));
        if base.is_empty() {
            continue;
        }
        if starts_with(&candidate, &base) {
            return Err(Touched {
                path: place.clone(),
                within: true,
            });
        }
        if reach == Reach::Tree && starts_with(&base, &candidate) {
            return Err(Touched {
                path: place.clone(),
                within: false,
            });
        }
    }
    Ok(())
}

/// The path with links of its existing part resolved; the missing rest (a
/// file about to be written) is appended as written.
fn effective(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(real) = std::fs::canonicalize(&current) {
            let mut real = real;
            for name in missing.iter().rev() {
                real.push(name);
            }
            return real;
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                current = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Components compared without regard to case.
fn key(path: &Path) -> Vec<String> {
    path.components()
        .filter(|component| !matches!(component, std::path::Component::CurDir))
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

fn starts_with(path: &[String], base: &[String]) -> bool {
    path.len() >= base.len() && path[..base.len()] == *base
}

/// Why the `se` tool refuses `arguments`, if it does.
pub(super) fn cli_refusal(set: &MaskSet, arguments: &[String]) -> Option<String> {
    let help = arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h");
    let words: Vec<&str> = arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| !argument.starts_with('-'))
        .collect();
    match words.as_slice() {
        ["masks", ..] if !help => {
            return Some(
                "`se masks` is for the user at a terminal or in the desktop app; list mask lists and add lists, entries and guarded folders with the `masks` tool".into(),
            )
        }
        ["elevated", ..] => {
            return Some("`se elevated` is an internal helper of the `shell` tool".into())
        }
        _ => {}
    }
    if set.is_empty() {
        return None;
    }
    if matches!(
        words.as_slice(),
        ["plugin", "uninstall" | "retire-installation", ..]
    ) {
        return Some(
            "removing the agent plugin is refused while a mask list is active; the user can run `se plugin uninstall` in a terminal".into(),
        );
    }
    arguments
        .iter()
        .any(|argument| crate::masks::refers_to_guarded(argument).is_some())
        .then(|| {
            "this command names Smart Explorer's own data or an agent host's configuration, which se tools leave alone while a mask list is active".into()
        })
}
