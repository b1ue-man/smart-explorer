//! Immutable folder names in one account/parent namespace. Missing objects
//! remain here as tombstones so a later sibling cannot inherit their alias.
use super::identity::invalid;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io;

pub(super) const VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FolderBinding {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) sync_name: String,
    pub(super) segment: String,
    /// Exact existing picker/cache locators that proved the same object.
    pub(super) aliases: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FolderBindings {
    pub(super) version: u32,
    pub(super) account: String,
    pub(super) parent: String,
    pub(super) folders: Vec<FolderBinding>,
}

impl FolderBindings {
    pub(super) fn empty(account: &str, parent: &str) -> Self {
        Self { version: VERSION, account: account.into(), parent: parent.into(), folders: Vec::new() }
    }

    pub(super) fn validate(&self, account: &str, parent: &str) -> io::Result<()> {
        if self.version != VERSION || self.account != account || self.parent != parent {
            return Err(invalid("Drive folder bindings have an unknown version or namespace"));
        }
        let mut objects = HashSet::new();
        let mut names = HashSet::new();
        let mut segments = HashSet::new();
        for folder in &self.folders {
            if folder.id.is_empty() || folder.title.is_empty()
                || !objects.insert((&folder.id, &folder.title))
                || !names.insert(&folder.sync_name)
            {
                return Err(invalid("Drive folder bindings contain conflicting identities"));
            }
            super::names::validate_component(&folder.sync_name)?;
            for segment in std::iter::once(&folder.segment).chain(&folder.aliases) {
                super::names::validate_component(segment)?;
                if !segments.insert(segment) || !segment_matches(segment, &folder.title, &folder.id)? {
                    return Err(invalid("Drive folder bindings contain conflicting locators"));
                }
            }
        }
        Ok(())
    }

    pub(super) fn by_segment(&self, segment: &str) -> Option<&FolderBinding> {
        self.folders.iter().find(|folder| folder.segment == segment || folder.aliases.iter().any(|alias| alias == segment))
    }

    pub(super) fn by_name(&self, name: &str) -> Option<&FolderBinding> {
        self.folders.iter().find(|folder| folder.sync_name == name)
    }

    pub(super) fn by_object(&self, title: &str, id: &str) -> Option<&FolderBinding> {
        self.folders.iter().find(|folder| folder.title == title && folder.id == id)
    }

    pub(super) fn bind_exact(
        &mut self, title: &str, id: &str, segment: &str, sync_name: &str,
    ) -> io::Result<FolderBinding> {
        if let Some(existing) = self.by_segment(segment) {
            if existing.id != id || existing.title != title {
                return Err(invalid("Drive locator is reserved for another folder identity"));
            }
            return Ok(existing.clone());
        }
        if let Some(index) = self.folders.iter().position(|folder| folder.title == title && folder.id == id) {
            self.folders[index].aliases.push(segment.to_string());
            return Ok(self.folders[index].clone());
        }
        if self.by_name(sync_name).is_some() {
            return Err(invalid("Drive logical name is reserved for another folder identity"));
        }
        let folder = FolderBinding {
            title: title.into(), id: id.into(), sync_name: sync_name.into(), segment: segment.into(), aliases: Vec::new(),
        };
        self.folders.push(folder.clone());
        Ok(folder)
    }
}

fn segment_matches(mut segment: &str, title: &str, id: &str) -> io::Result<bool> {
    if super::names::decode(segment)? == title { return Ok(true); }
    while let Some((plain, prefix)) = super::duplicates::parse_marker(segment) {
        if !id.starts_with(prefix) { return Ok(false); }
        segment = plain;
        if super::names::decode(segment)? == title { return Ok(true); }
    }
    Ok(false)
}
