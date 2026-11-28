//! Platform-free part of the mask lists: model, folding, matching,
//! placeholders, masking set, stream masker, store document and its sealing.
pub(crate) mod admin;
pub(crate) mod doc;
pub(crate) mod fold;
pub(crate) mod fuzzy;
pub(crate) mod mask_set;
pub(crate) mod matcher;
pub(crate) mod model;
pub(crate) mod placeholder;
pub(crate) mod rights;
pub(crate) mod scope;
pub(crate) mod stream;
pub(crate) mod tables;

#[cfg(test)]
mod admin_tests;
#[cfg(test)]
mod fuzzy_tests;
#[cfg(test)]
mod mask_set_tests;
#[cfg(test)]
mod stream_tests;
