//! Durable folder aliases are scoped by job and both endpoint identities.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderAliasScope {
    pub job: String,
    pub source_namespace: String,
    pub target_namespace: String,
    pub target_root: String,
    pub native_target: bool,
    pub vault_locations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderAlias {
    pub wanted: String,
    pub relative: String,
    /// Receipt produced by the exclusive reservation. A pending record has
    /// no receipt and cannot adopt an existing folder merely because empty.
    pub identity: Option<String>,
    pub stage: String,
    pub published: bool,
    pub move_source: bool,
    /// Atomic mkdir ACK owns this running operation; without an object ID
    /// a later process must leave the published alias unconfirmed.
    pub direct: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderAliases {
    pub version: u32,
    pub scope: FolderAliasScope,
    pub roots: BTreeMap<String, FolderAlias>,
}

impl FolderAliases {
    pub fn new(scope: FolderAliasScope) -> Self {
        Self { version: 1, scope, roots: BTreeMap::new() }
    }
}
