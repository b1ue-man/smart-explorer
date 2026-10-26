//! Read-only administrative trust anchors; no user updater or alternate path.
use super::{super::platform, denied};
use crate::login::core::{
    manifest::Manifest,
    model::{Policy, MAX_POLICY},
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
};
pub(crate) struct Paths {
    pub config: PathBuf,
    pub releases: PathBuf,
    pub image_name: &'static str,
}
pub(crate) struct Installation {
    pub policy: Policy,
    pub manifest: Manifest,
    pub image: platform::ProtectedFile,
    pub image_path: PathBuf,
}

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(
    mut file: File,
    max: usize,
) -> io::Result<T> {
    if file.metadata()?.len() > max as u64 {
        return Err(denied("protected login record exceeds limit"));
    }
    let mut bytes = Vec::new();
    file.by_ref().take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(denied("protected login record exceeds limit"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
pub(crate) fn policy() -> io::Result<Policy> {
    let paths = platform::paths()?;
    let pinned = platform::open_protected(&paths.config.join("policy.json"), false)?;
    let policy: Policy = read_json(pinned.file.try_clone()?, MAX_POLICY)?;
    policy.validate().map_err(denied)?;
    platform::validate_gateway_identity(&policy.gateway_identity)?;
    Ok(policy)
}
pub(crate) fn load() -> io::Result<Installation> {
    let policy = policy()?;
    let paths = platform::paths()?;
    let pinned = platform::open_protected(&paths.config.join("manifest.json"), false)?;
    let manifest: Manifest = read_json(pinned.file.try_clone()?, 4096)?;
    manifest.validate()?;
    if manifest.version != env!("CARGO_PKG_VERSION") {
        return Err(denied(
            "protected login manifest version does not match the running release",
        ));
    }
    let image_path = paths
        .releases
        .join(&manifest.generation)
        .join(paths.image_name);
    let image = platform::open_protected(&image_path, true)?;
    Ok(Installation {
        policy,
        manifest,
        image,
        image_path,
    })
}
pub(crate) fn verify_hash(file: &File, expected: &str) -> io::Result<()> {
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    let actual: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    file.seek(SeekFrom::Start(0))?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(denied("protected login executable SHA-256 mismatch"));
    }
    Ok(())
}
/// Administrative offline candidate validation. No endpoint or service is started.
pub(crate) fn validate_candidate(
    version: &str,
    expected: &str,
    records: &[PathBuf],
) -> io::Result<()> {
    use crate::login::core::{
        gateway_types::{GatewayNetwork, MAX_NETWORK},
        machine::MachineIdentity,
    };
    if records.len() != 4 || version != env!("CARGO_PKG_VERSION") {
        return Err(denied(
            "candidate executable release does not match the requested version",
        ));
    }
    let policy_pin = platform::open_protected(&records[0], false)?;
    let policy: Policy = read_json(policy_pin.file.try_clone()?, MAX_POLICY)?;
    policy.validate().map_err(denied)?;
    let key_pin = platform::open_protected(&records[1], false)?;
    let identity: MachineIdentity = read_json(key_pin.file.try_clone()?, 8192)?;
    let _key = identity.key_for_policy(&policy)?;
    let network_pin = platform::open_protected(&records[2], false)?;
    let network: GatewayNetwork = read_json(network_pin.file.try_clone()?, MAX_NETWORK)?;
    network.validate()?;
    let manifest_pin = platform::open_protected(&records[3], false)?;
    let manifest: Manifest = read_json(manifest_pin.file.try_clone()?, 4096)?;
    manifest.validate()?;
    if manifest.version != version
        || !manifest.sha256.eq_ignore_ascii_case(expected)
        || manifest.generation != format!("{}-{}", version, &manifest.sha256[..16])
    {
        return Err(denied(
            "candidate manifest does not bind the requested release/hash/generation",
        ));
    }
    let image = platform::open_protected(&std::env::current_exe()?, true)?;
    verify_hash(&image.file, expected)
}
pub(crate) fn running() -> io::Result<Installation> {
    let install = load()?;
    if !same_file::is_same_file(std::env::current_exe()?, &install.image_path)? {
        return Err(denied(
            "login service is not the protected manifest release image",
        ));
    }
    verify_hash(&install.image.file, &install.manifest.sha256)?;
    Ok(install)
}
