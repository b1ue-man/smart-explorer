//! Literal sync components, independent of native paths and the Agent wire grammar.
use std::io;

const MAX_PATH_BYTES: usize = 32 * 1024;

/// `/` separates literal provider names. No decoding, normalization or native
/// joining is permitted here: `:`, `\\` and `%` retain their provider meaning.
/// Real I/O still requires sync_path, endpoint guards and the backend's limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SyncRelativePath<'a> {
    literal: &'a str,
}

impl<'a> SyncRelativePath<'a> {
    pub(super) fn parse(literal: &'a str) -> io::Result<Self> {
        if literal.len() > MAX_PATH_BYTES {
            return Err(invalid("sync relative path exceeds its byte budget"));
        }
        // Walk and index depth budgets belong to their respective consumers.
        for name in literal.split('/') {
            validate_component(name)?;
        }
        Ok(Self { literal })
    }

    pub(super) fn as_str(self) -> &'a str {
        self.literal
    }
}

pub(super) fn validate_component(name: &str) -> io::Result<()> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains('/') || name.contains('\0') {
        return Err(invalid("sync name is not one literal path component"));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
