//! Explicit administrative bundles and a socket-owning CLI login session,
//! opened for a paired device (E22) or from an explicit machine target file.
use crate::{
    daemon::{LoginCommand, LoginViewer},
    login::{GatewayNetwork, GatewayTarget, PeerGrant},
    share::login_transport::{self, OpenChoice},
};
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{self, BufRead, Read},
    path::{Path, PathBuf},
    time::Duration,
};
/// Update commands printed for an installed gateway. The Linux state directory
/// is `state=` in native/install-login-service.sh (line 13). The Windows command
/// omits `-MachineIdentityFile`, so the installed machine key stays in use.
const LINUX_NETWORK_UPDATE: &str = "install-login-service.sh install RELEASE_IMAGE APPROVED_HASH RELEASE_VERSION POLICY_JSON /var/lib/smart-explorer-login-gateway/machine-key.json BUNDLE/gateway-network.json";
const WINDOWS_NETWORK_UPDATE: &str = "install-login-service.ps1 -Image RELEASE_EXE -ExpectedSha256 APPROVED_HASH -Version RELEASE_VERSION -PolicyFile POLICY_JSON -NetworkFile BUNDLE/gateway-network.json";
#[derive(Args)]
pub(super) struct LoginArgs {
    #[command(subcommand)]
    command: LoginCliCommand,
}
#[derive(Subcommand)]
enum LoginCliCommand {
    /// Read installed records, administrative rights and platform limits.
    Status {
        #[arg(long)]
        json: bool,
    },
    Sessions {
        #[arg(long)]
        json: bool,
    },
    Session {
        id: String,
        #[arg(long)]
        json: bool,
    },
    Close {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Generate four private bundle files; never installs or enables services.
    Provision {
        directory: PathBuf,
        #[arg(long)]
        grants_file: PathBuf,
        #[arg(long)]
        network_file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Write only gateway-network.json with the relays of the configured Share server; keeps the installed machine identity.
    ProvisionNetwork {
        directory: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Inspect an explicitly supplied public machine target; no peer lookup.
    Target {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Open the login screen of a paired device: `se login open <device>`.
    /// The device sends its machine target over the pinned Share session once
    /// it allows this device „Bildschirm steuern“; no file, no pin
    /// acknowledgement. The administrative form `se login open TARGET_FILE
    /// --target-user ID --acknowledge-machine-pin` stays. Owns a viewer until
    /// stdin EOF/stop, reads bounded LoginCommand JSON lines from stdin and
    /// outputs metadata only, never pixel bytes or credential data.
    Open {
        /// Paired device (device ID, contact ID or name); with
        /// --acknowledge-machine-pin a public gateway-target.json.
        target: String,
        /// Host account (Windows SID, Linux UID); a paired device names it.
        #[arg(long)]
        target_user: Option<String>,
        #[arg(long)]
        acknowledge_machine_pin: bool,
        #[arg(long)]
        json: bool,
    },
}
pub(super) fn run(args: LoginArgs) -> Result<i32, String> {
    match args.command {
        LoginCliCommand::Status { json } => output(
            json,
            serde_json::to_value(crate::daemon::login_status()?).map_err(text)?,
        ),
        LoginCliCommand::Sessions { json } => {
            output(json, json!({"sessions":crate::daemon::login_sessions()?}))
        }
        LoginCliCommand::Session { id, json } => {
            output(json, json!({"session":crate::daemon::login_session(id)?}))
        }
        LoginCliCommand::Close { id, json } => {
            output(json, json!({"stopped":crate::daemon::login_stop(id)?}))
        }
        LoginCliCommand::Target { file, json } => {
            let target = target_file(&file)?;
            output(json, json!({"target":target,"pinning_required":true}));
        }
        LoginCliCommand::Provision {
            directory,
            grants_file,
            network_file,
            json,
        } => {
            let grants: Vec<PeerGrant> =
                read_json(&grants_file, crate::login::core::model::MAX_POLICY)?;
            let network: GatewayNetwork = read_json(
                &network_file,
                crate::login::core::gateway_types::MAX_NETWORK,
            )?;
            let bundle = crate::login::provision(crate::login::ProvisionRequest {
                directory,
                grants,
                network,
            })
            .map_err(text)?;
            output(
                json,
                json!({"bundle":bundle,"installed":false,
                "windows_install":"install-login-service.ps1 -Image RELEASE_EXE -ExpectedSha256 APPROVED_HASH -Version RELEASE_VERSION -PolicyFile BUNDLE/policy.json -MachineIdentityFile BUNDLE/machine-key.json -NetworkFile BUNDLE/gateway-network.json",
                "linux_install":"install-login-service.sh install RELEASE_IMAGE APPROVED_HASH RELEASE_VERSION BUNDLE/policy.json BUNDLE/machine-key.json BUNDLE/gateway-network.json"}),
            );
        }
        LoginCliCommand::ProvisionNetwork { directory, json } => {
            let provisioned = crate::login::provision_network(directory).map_err(text)?;
            output(
                json,
                json!({"network_file":provisioned.network_file,"network":provisioned.network,
                "installed":false,"linux_update":LINUX_NETWORK_UPDATE,
                "windows_update":WINDOWS_NETWORK_UPDATE}),
            );
        }
        LoginCliCommand::Open {
            target,
            target_user,
            acknowledge_machine_pin,
            json,
        } => {
            let file = PathBuf::from(&target);
            let choice = login_transport::login_open_choice(
                &target,
                acknowledge_machine_pin,
                file.is_file(),
            )
            .map_err(text)?;
            let (mut viewer, paired) = match choice {
                OpenChoice::File => {
                    let user = target_user.filter(|user| !user.is_empty()).ok_or_else(|| {
                        "--target-user is required with a target file".to_string()
                    })?;
                    let viewer =
                        LoginViewer::open(target_file(&file)?, user, acknowledge_machine_pin)?;
                    (viewer, Value::Null)
                }
                OpenChoice::Device(paired) => {
                    let user = target_user
                        .filter(|user| !user.is_empty())
                        .unwrap_or_else(|| paired.target_user.clone());
                    let viewer = LoginViewer::open(paired.target.clone(), user, true)?;
                    (viewer, paired_json(&paired))
                }
            };
            let started = viewer.command(LoginCommand::Start)?;
            let pixel_bytes = started
                .image
                .as_ref()
                .map_or(0, |image| image.image().data().len());
            output(
                json,
                json!({"reply":started.reply,"pixel_bytes":pixel_bytes,"paired":paired}),
            );
            if viewer.view().challenge.is_none() {
                return Ok(0);
            }
            let (send, input) = std::sync::mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("login-cli-input".into())
                .spawn(move || {
                    let stdin = io::stdin();
                    let mut input = stdin.lock();
                    loop {
                        let line = bounded_line(&mut input);
                        let end = !matches!(&line, Ok(Some(_)));
                        if send.send(line).is_err() || end {
                            break;
                        }
                    }
                })
                .map_err(text)?;
            loop {
                if let Some(end) = viewer.poll_end()? {
                    output(json, json!({"end":end}));
                    break;
                }
                match input.recv_timeout(Duration::from_millis(100)) {
                    Ok(Ok(Some(bytes))) => {
                        if bytes.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        let command: LoginCommand = serde_json::from_slice(&bytes).map_err(text)?;
                        let frame = viewer.command(command)?;
                        let pixel_bytes = frame
                            .image
                            .as_ref()
                            .map_or(0, |image| image.image().data().len());
                        output(
                            json,
                            json!({"reply":frame.reply,"pixel_bytes":pixel_bytes,"pixel_stream_in_json":false}),
                        );
                        if viewer.view().challenge.is_none() {
                            break;
                        }
                    }
                    Ok(Ok(None)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Err(error)) => return Err(error),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            viewer.close();
        }
    }
    Ok(0)
}
/// The paired device of a device-form open (E22); no key material beyond the
/// public machine pin.
fn paired_json(paired: &login_transport::PairedLoginTarget) -> Value {
    json!({"device_id":paired.device_id,"device_name":paired.device_name,
        "machine_id":paired.target.machine_id,"machine_public_key":paired.target.public_key,
        "target_user":paired.target_user,"received_at":paired.received_at})
}
pub(crate) fn target_file(path: &Path) -> Result<GatewayTarget, String> {
    let target: GatewayTarget = read_json(path, crate::login::core::gateway_types::MAX_NETWORK)?;
    target.validate().map_err(text)?;
    Ok(target)
}
fn read_json<T: DeserializeOwned>(path: &Path, max: usize) -> Result<T, String> {
    let file = File::open(path).map_err(text)?;
    if file.metadata().map_err(text)?.len() > max as u64 {
        return Err("login input JSON exceeds its record limit".into());
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(text)?;
    if bytes.len() > max {
        return Err("login input JSON exceeds its record limit".into());
    }
    serde_json::from_slice(&bytes).map_err(text)
}
fn bounded_line(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let read = input
        .take(crate::login::core::model::MAX_CONTROL as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(text)?;
    if bytes.len() > crate::login::core::model::MAX_CONTROL {
        return Err("login command exceeds 64 KiB".into());
    }
    Ok((read != 0).then_some(bytes))
}
fn output(compact: bool, value: Value) {
    if compact {
        println!("{value}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
        );
    }
}
fn text(error: impl ToString) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;
