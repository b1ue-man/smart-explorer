//! New boot-gateway identity, never copied from a user daemon's profile.
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MachineIdentity {
    pub format: u32,
    pub machine_id: String,
    pub public_key: String,
    /// Store only in the dedicated gateway's private state directory; never log.
    pub secret_key: String,
}
impl MachineIdentity {
    pub(crate) fn generate() -> std::io::Result<Self> {
        let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::getrandom(bytes.as_mut())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let key = iroh::SecretKey::from_bytes(&bytes);
        let public_key = key.public().to_string();
        let secret_key = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            format: 1,
            machine_id: format!("login:{public_key}"),
            public_key,
            secret_key,
        })
    }
    pub(crate) fn key_for_policy(
        &self,
        policy: &super::model::Policy,
    ) -> std::io::Result<iroh::SecretKey> {
        if self.machine_id != policy.machine_id || self.public_key != policy.machine_public_key {
            return Err(std::io::Error::other(
                "boot gateway identity is not the administratively approved machine",
            ));
        }
        self.key()
    }
    pub(crate) fn key(&self) -> std::io::Result<iroh::SecretKey> {
        if self.format != 1 || self.secret_key.len() != 64 {
            return Err(std::io::Error::other("invalid machine identity"));
        }
        let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
        for (index, chunk) in self.secret_key.as_bytes().chunks_exact(2).enumerate() {
            let text = std::str::from_utf8(chunk)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            bytes[index] = u8::from_str_radix(text, 16)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        let key = iroh::SecretKey::from_bytes(&bytes);
        if key.public().to_string() != self.public_key
            || self.machine_id != format!("login:{}", self.public_key)
        {
            return Err(std::io::Error::other(
                "machine identity public key mismatch",
            ));
        }
        Ok(key)
    }
}
impl Drop for MachineIdentity {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.secret_key.zeroize();
    }
}
