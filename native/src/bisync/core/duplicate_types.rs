//! Duplicate file contents are compared independently of timestamps and policy.
use std::collections::BTreeSet;
use super::types::Sig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVariant {
    pub id: Option<String>,
    pub signature: Sig,
    pub content_size: u64,
    pub content_md5: String,
}

impl FileVariant {
    pub fn same_content(&self, other: &Self) -> bool {
        self.content_size == other.content_size && self.content_md5 == other.content_md5
    }
}

#[derive(Clone, Debug, Default)]
pub struct DuplicateConflict {
    pub a: Vec<FileVariant>,
    pub b: Vec<FileVariant>,
}

impl DuplicateConflict {
    pub fn variants(&self, keep_a: bool) -> &[FileVariant] {
        if keep_a { &self.a } else { &self.b }
    }

    /// Equal-content copies do not require the user to choose an object ID.
    pub fn needs_variant_choice(&self, keep_a: bool) -> bool {
        let variants = self.variants(keep_a);
        variants.first().is_some_and(|first| variants.iter().any(|v| !v.same_content(first)))
    }

    /// Exactly one shared content, even if either side has several copies.
    pub(crate) fn common_choice(&self) -> Option<(&FileVariant, &FileVariant)> {
        let shared: BTreeSet<_> = self.a.iter()
            .filter(|a| self.b.iter().any(|b| a.same_content(b)))
            .map(|v| (v.content_size, v.content_md5.as_str())).collect();
        if shared.len() != 1 { return None; }
        let a = self.a.iter().find(|v| shared.contains(&(v.content_size, v.content_md5.as_str())))?;
        Some((a, self.b.iter().find(|b| a.same_content(b))?))
    }

    pub(crate) fn redundant_count(&self) -> u64 {
        self.a.len().saturating_sub(1) as u64 + self.b.len().saturating_sub(1) as u64
    }
}
