//! Durable, exact same-account aliases. No locator or credential is inferred.
use super::incremental::SyncEndpoints;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Binding {
    pub pair: String,
    pub lock: String,
    pub identities: [String; 2],
    pub roots: [String; 2],
}

impl Binding {
    pub(super) fn new(identities: [String; 2], roots: [String; 2]) -> Self {
        let parts = [(&*identities[0], &*roots[0]), (&*identities[1], &*roots[1])];
        Self {
            pair: super::persistence::pair_id_parts(parts),
            lock: lock_id(parts),
            identities,
            roots,
        }
    }
    pub(super) fn current(endpoints: SyncEndpoints<'_>) -> Self {
        Self::new(
            [endpoints.a.state_identity(), endpoints.b.state_identity()],
            [endpoints.root_a.to_string(), endpoints.root_b.to_string()],
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Aliases {
    pub format: u8,
    pub current: Binding,
    pub previous: Vec<Binding>,
    pub complete: bool,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "backend identity alias is not exactly bound to this pair",
    )
}
fn path(pair: &str) -> io::Result<PathBuf> {
    super::version_manifest::validate_pair(pair)?;
    Ok(super::replica_state::pair_dir(pair).join("identity-aliases.json"))
}

pub(super) fn read(pair: &str) -> io::Result<Option<Aliases>> {
    let text = match crate::support_dirs::read_private_text(&path(pair)?, 2 * 1024 * 1024) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let aliases: Aliases = serde_json::from_str(&text).map_err(io::Error::other)?;
    if aliases.format != 1 || aliases.previous.len() > 80 {
        return Err(invalid());
    }
    for binding in std::iter::once(&aliases.current).chain(&aliases.previous) {
        if binding
            .identities
            .iter()
            .any(|id| id.is_empty() || id.len() > 4096)
            || binding
                .roots
                .iter()
                .any(|root| root.len() > 64 * 1024 || root.contains('\0'))
            || *binding != Binding::new(binding.identities.clone(), binding.roots.clone())
            || binding.roots != aliases.current.roots
        {
            return Err(invalid());
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for binding in &aliases.previous {
        if binding.pair == aliases.current.pair || !seen.insert(&binding.pair) {
            return Err(invalid());
        }
    }
    for side in 0..2 {
        let identities: std::collections::BTreeSet<_> = aliases
            .previous
            .iter()
            .map(|binding| &binding.identities[side])
            .filter(|identity| *identity != &aliases.current.identities[side])
            .collect();
        if identities.len() > 8 {
            return Err(invalid());
        }
    }
    if aliases.current.pair != pair && !aliases.previous.iter().any(|binding| binding.pair == pair)
    {
        return Err(invalid());
    }
    Ok(Some(aliases))
}

pub(super) fn save(aliases: &Aliases) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(aliases).map_err(io::Error::other)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid());
    }
    crate::support_dirs::write_private_atomic(&path(&aliases.current.pair)?, &bytes)
}
pub(super) fn save_backlinks(aliases: &Aliases) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(aliases).map_err(io::Error::other)?;
    for binding in &aliases.previous {
        match read(&binding.pair)? {
            Some(old) if old.current != aliases.current => return Err(invalid()),
            _ => {}
        }
        crate::support_dirs::write_private_atomic(&path(&binding.pair)?, &bytes)?;
    }
    Ok(())
}

pub(super) fn family(pair: &str) -> io::Result<Vec<String>> {
    let Some(aliases) = read(pair)? else {
        return Ok(vec![pair.to_string()]);
    };
    if !aliases.complete {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "identity migration is incomplete",
        ));
    }
    Ok(std::iter::once(aliases.current.pair)
        .chain(aliases.previous.into_iter().map(|b| b.pair))
        .collect())
}

pub(super) fn version_matches(
    pair: &str,
    stored_pair: &str,
    backend: &str,
    root: &str,
    side: usize,
    current_backend: &str,
) -> io::Result<bool> {
    let Some(aliases) = read(pair)? else {
        return Ok(false);
    };
    if !aliases.complete
        || aliases.current.identities[side] != current_backend
        || aliases.current.roots[side] != root
    {
        return Ok(false);
    }
    Ok(aliases.previous.iter().any(|binding| {
        binding.pair == stored_pair
            && binding.identities[side] == backend
            && binding.roots[side] == root
    }))
}

pub(super) fn lock_matches(pair: &str, stored_lock: &str, current_lock: &str) -> io::Result<bool> {
    if stored_lock == current_lock {
        return Ok(true);
    }
    let Some(aliases) = read(pair)? else {
        return Ok(false);
    };
    Ok(aliases.complete
        && aliases.current.lock == current_lock
        && aliases
            .previous
            .iter()
            .any(|binding| binding.pair == pair && binding.lock == stored_lock))
}

pub(super) fn replica_matches(
    pair: &str,
    stored: &str,
    actual: &str,
    side: usize,
) -> io::Result<bool> {
    if stored == actual {
        return Ok(true);
    }
    let Some(aliases) = read(pair)? else {
        return Ok(false);
    };
    if !aliases.complete {
        return Ok(false);
    }
    let current = &aliases.current;
    if actual
        != format!(
            "location:{}:{}",
            current.identities[side], current.roots[side]
        )
    {
        return Ok(false);
    }
    Ok(aliases.previous.iter().any(|binding| {
        binding.pair == pair
            && stored
                == format!(
                    "location:{}:{}",
                    binding.identities[side], binding.roots[side]
                )
    }))
}

/// Identical length-prefixed unordered lock identity used by PairLock.
pub(super) fn lock_id(mut parts: [(&str, &str); 2]) -> String {
    parts.sort_unstable();
    let mut hash = 0xcbf29ce484222325u64;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    add(b"smart-explorer/bisync-pair-lock/v1");
    for (identity, root) in parts {
        add(&(identity.len() as u64).to_be_bytes());
        add(identity.as_bytes());
        add(&(root.len() as u64).to_be_bytes());
        add(root.as_bytes());
    }
    format!("{hash:016x}")
}
