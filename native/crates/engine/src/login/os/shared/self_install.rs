//! The elevated step of the login self-setup (E21, step 2 (b), (c), (f)):
//! `se rdp host-setup --system login-service:<sha256>` or
//! `--system login-service-remove`, run as root/administrator directly or
//! through pkexec/UAC. It takes nothing from the caller but the hash the
//! approval names: image, version, policy, machine identity and network are
//! read here, the checked-in installer runs with fixed arguments from a new
//! protected directory, and every check fails before a record changes.
use super::{denied, installation, nonce, self_channel::Record};
use crate::login::{
    core::{
        gateway_types::{GatewayNetwork, MAX_NETWORK},
        machine::MachineIdentity,
        model::{Policy, MAX_POLICY, VERSION},
        self_plan::{exit, image_binding, install_policy, state_for_exit, Step},
    },
    os::platform,
};
use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

/// How the worker reaches a privileged step.
pub enum Elevation {
    /// This process is root or an elevated administrator / LocalSystem.
    InProcess,
    /// pkexec (Linux) or UAC (Windows) runs this `se` with the step.
    External(PathBuf),
}

/// The fixed installer call; the platform adapter turns it into argv.
pub enum ScriptCall<'a> {
    SelfInstall {
        image: &'a Path,
        sha256: &'a str,
        version: &'a str,
        policy: &'a Path,
        key: &'a Path,
        network: &'a Path,
    },
    Uninstall,
}

const POLICY: &str = "policy.json";
const KEY: &str = "machine-key.json";
const NETWORK: &str = "gateway-network.json";

/// The program the service runs: `beside` itself when it is the app,
/// otherwise the app next to it (the app handles the login switches).
pub fn app_image(beside: &Path) -> PathBuf {
    if beside
        .file_name()
        .is_some_and(|name| name == platform::APP_FILE)
    {
        beside.to_path_buf()
    } else {
        beside.with_file_name(platform::APP_FILE)
    }
}

/// The installed policy without the live gateway-identity check: after a
/// self-removal on Windows the gateway service SID no longer resolves, and
/// its retained grants must survive the next setup.
pub fn existing_policy() -> io::Result<Option<Policy>> {
    let paths = platform::paths()?;
    let pinned = match platform::open_protected(&paths.config.join(POLICY), false) {
        Ok(pinned) => pinned,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let policy: Policy = installation::read_json(pinned.file.try_clone()?, MAX_POLICY)?;
    policy.validate().map_err(denied)?;
    Ok(Some(policy))
}

fn direct_only() -> GatewayNetwork {
    GatewayNetwork {
        format: VERSION,
        use_default_relays: false,
        relay_urls: Vec::new(),
        relay_pins: Vec::new(),
        allow_plaintext_relays: false,
    }
}

/// The retained identity (or a new one), the policy and the network.
fn records() -> io::Result<(Zeroizing<Vec<u8>>, Vec<u8>, Vec<u8>)> {
    let existing = existing_policy()?;
    let identity = match platform::read_self_record(Record::MachineKey, 8192)? {
        Some(bytes) => {
            let bytes = Zeroizing::new(bytes);
            let identity: MachineIdentity = serde_json::from_slice(&bytes)
                .map_err(|_| denied("installed machine identity is invalid"))?;
            identity.key()?;
            identity
        }
        None => MachineIdentity::generate()?,
    };
    let policy = install_policy(existing.as_ref(), &identity);
    policy.validate().map_err(denied)?;
    // The worker sends the configured server's relays right after setup;
    // until then an installed network stays, otherwise direct paths only.
    let network = platform::read_self_record(Record::Network, MAX_NETWORK)?
        .and_then(|bytes| serde_json::from_slice::<GatewayNetwork>(&bytes).ok())
        .filter(|network| network.validate().is_ok())
        .unwrap_or_else(direct_only);
    let key = Zeroizing::new(serde_json::to_vec(&identity).map_err(io::Error::other)?);
    let policy = serde_json::to_vec(&policy).map_err(io::Error::other)?;
    let network = serde_json::to_vec(&network).map_err(io::Error::other)?;
    Ok((key, policy, network))
}

/// A new protected directory (root 0700, or SYSTEM/Administrators only);
/// every file in it is removed when the step ends.
struct Stage {
    dir: PathBuf,
    files: Vec<&'static str>,
}

impl Stage {
    fn create(records: &[(&'static str, &[u8])]) -> io::Result<Self> {
        let dir = platform::stage_root()?.join(format!("smart-explorer-login-self-{}", nonce()?));
        platform::write_provision_bundle(&dir, records)?;
        Ok(Self {
            files: records.iter().map(|(name, _)| *name).collect(),
            dir,
        })
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    /// Copies `source` into the stage and returns the copy's own hash.
    fn copy_image(&mut self, source: &File) -> io::Result<String> {
        self.files.push(platform::STAGE_IMAGE);
        let mut target = platform::create_stage_file(&self.path(platform::STAGE_IMAGE))?;
        let mut reader = source.try_clone()?;
        reader.seek(SeekFrom::Start(0))?;
        io::copy(&mut reader, &mut target)?;
        target.flush()?;
        target.sync_all()?;
        installation::sha256_hex(&target)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        for name in &self.files {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
        let _ = std::fs::remove_dir(&self.dir);
    }
}

fn fail(code: i32) -> impl Fn(io::Error) -> i32 {
    move |error| {
        eprintln!("Maschinenanmeldung, Einrichtungsschritt: {error}");
        code
    }
}

fn install(expected: &str) -> Result<(), i32> {
    let image = app_image(&std::env::current_exe().map_err(fail(exit::IMAGE_MISSING))?);
    let source = platform::open_image(&image).map_err(fail(exit::IMAGE_MISSING))?;
    let source_hash = installation::sha256_hex(&source).map_err(fail(exit::IMAGE_MISSING))?;
    image_binding(expected, &source_hash, &source_hash).map_err(|_| exit::HASH_MISMATCH)?;
    let (key, policy, network) = records().map_err(fail(exit::STAGE_FAILED))?;
    let mut stage = Stage::create(&[
        (platform::SCRIPT_NAME, platform::SCRIPT.as_bytes()),
        (POLICY, &policy),
        (KEY, key.as_slice()),
        (NETWORK, &network),
    ])
    .map_err(fail(exit::STAGE_FAILED))?;
    let staged = stage
        .copy_image(&source)
        .map_err(fail(exit::STAGE_FAILED))?;
    // Re-checked on the protected copy before the installer replaces
    // anything; the installer checks it once more before the swap.
    image_binding(expected, &source_hash, &staged).map_err(|_| exit::HASH_MISMATCH)?;
    let (image, policy, key, network) = (
        stage.path(platform::STAGE_IMAGE),
        stage.path(POLICY),
        stage.path(KEY),
        stage.path(NETWORK),
    );
    let call = ScriptCall::SelfInstall {
        image: &image,
        sha256: expected,
        version: env!("CARGO_PKG_VERSION"),
        policy: &policy,
        key: &key,
        network: &network,
    };
    let code =
        platform::run_install_script(&stage.dir, &call).map_err(fail(exit::SCRIPT_FAILED))?;
    if code != 0 {
        eprintln!("Maschinenanmeldung: Installationsskript endete mit Code {code}");
        return Err(exit::SCRIPT_FAILED);
    }
    Ok(())
}

/// Only when no grant is left at all (step 2 (f)); the installer keeps the
/// policy, the machine identity, the network and the release generations.
fn remove() -> Result<(), i32> {
    if existing_policy()
        .map_err(fail(exit::STAGE_FAILED))?
        .is_some_and(|policy| !policy.grants.is_empty())
    {
        return Err(exit::GRANTS_REMAIN);
    }
    let stage = Stage::create(&[(platform::SCRIPT_NAME, platform::SCRIPT.as_bytes())])
        .map_err(fail(exit::STAGE_FAILED))?;
    let code = platform::run_install_script(&stage.dir, &ScriptCall::Uninstall)
        .map_err(fail(exit::SCRIPT_FAILED))?;
    if code != 0 {
        eprintln!("Maschinenanmeldung: Entfernen endete mit Code {code}");
        return Err(exit::SCRIPT_FAILED);
    }
    Ok(())
}

/// `se rdp host-setup --system login-service…`: the line to print and the
/// exit code (`self_plan::state_for_exit` reads it in the worker).
pub fn run_system_step(step: &str) -> (String, i32) {
    let parsed = Step::parse(step);
    let removing = parsed == Some(Step::Remove);
    let code = match parsed {
        None => exit::INVALID_STEP,
        Some(_) if !platform::privileged() => exit::NOT_PRIVILEGED,
        Some(Step::Install { sha256 }) => install(&sha256).err().unwrap_or(exit::DONE),
        Some(Step::Remove) => remove().err().unwrap_or(exit::DONE),
    };
    let state = serde_json::to_string(&state_for_exit(code, removing))
        .unwrap_or_else(|_| "\"unknown\"".into());
    (format!("login_state={state}"), code)
}
