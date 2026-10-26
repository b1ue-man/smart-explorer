//! The administrative manifest points only inside the protected release root.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub format: u32,
    pub version: String,
    pub generation: String,
    pub sha256: String,
}
impl Manifest {
    pub(crate) fn validate(&self) -> std::io::Result<()> {
        if self.format != 1
            || self.version.is_empty()
            || self.version.len() > 64
            || !self
                .version
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'.')
            || self.generation.is_empty()
            || self.generation.len() > 96
            || !self
                .generation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(std::io::Error::other(
                "invalid protected login release manifest",
            ));
        }
        Ok(())
    }
}
