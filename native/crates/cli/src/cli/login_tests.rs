//! `se login provision-network` (L-B2, E24; suite expectation (f) of
//! `f-lokal-2`): the command takes one directory, nothing of `provision`'s
//! inputs, and prints update commands the installation scripts understand.
use std::path::PathBuf;

use clap::{CommandFactory, Parser};

use super::{LoginCliCommand, LINUX_NETWORK_UPDATE, WINDOWS_NETWORK_UPDATE};
use crate::cli::{Cli, Command};

const WINDOWS_SCRIPT: &str = include_str!("../../../../install-login-service.ps1");
const LINUX_SCRIPT: &str = include_str!("../../../../install-login-service.sh");

fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("se").chain(arguments.iter().copied()))
}

fn login_command(arguments: &[&str]) -> LoginCliCommand {
    match parse(arguments).unwrap_or_else(|error| panic!("{arguments:?}: {error}")) {
        Cli {
            command: Command::Login(args),
        } => args.command,
        _ => panic!("{arguments:?}: not a login command"),
    }
}

#[test]
fn lokal_task_login_provision_network_parses_one_directory_and_json() {
    match login_command(&["login", "provision-network", "/srv/bundle", "--json"]) {
        LoginCliCommand::ProvisionNetwork { directory, json } => {
            assert_eq!(directory, PathBuf::from("/srv/bundle"));
            assert!(json);
        }
        _ => panic!("not the provision-network command"),
    }
    match login_command(&["login", "provision-network", "/srv/bundle"]) {
        LoginCliCommand::ProvisionNetwork { json, .. } => assert!(!json),
        _ => panic!("not the provision-network command"),
    }
    // Nothing of `provision`'s inputs: it writes the one file and creates no
    // identity, so a grants file or a network file is no argument of it.
    for rejected in [
        &["login", "provision-network"][..],
        &[
            "login",
            "provision-network",
            "/srv/bundle",
            "--grants-file",
            "g.json",
        ],
        &[
            "login",
            "provision-network",
            "/srv/bundle",
            "--network-file",
            "n.json",
        ],
        &["login", "provision-network", "/srv/a", "/srv/b"],
        &["login", "provision", "/srv/bundle"],
    ] {
        assert!(parse(rejected).is_err(), "unexpectedly parsed {rejected:?}");
    }
    let mut command = Cli::command();
    let help = command
        .find_subcommand_mut("login")
        .expect("login command")
        .find_subcommand_mut("provision-network")
        .expect("provision-network command")
        .render_long_help()
        .to_string();
    assert!(help.contains("gateway-network.json"), "{help}");
}

/// The update commands the output prints name parameters the scripts have, and
/// the gateway state directory the Linux script uses.
#[test]
fn lokal_task_provision_network_update_commands_match_the_install_scripts() {
    for parameter in WINDOWS_NETWORK_UPDATE
        .split_whitespace()
        .filter(|word| word.starts_with('-'))
    {
        assert!(
            WINDOWS_SCRIPT.contains(&format!("${}", &parameter[1..])),
            "install-login-service.ps1 has no parameter {parameter}"
        );
    }
    // The installed machine key stays: no -MachineIdentityFile is passed.
    assert!(!WINDOWS_NETWORK_UPDATE.contains("-MachineIdentityFile"));
    assert!(WINDOWS_NETWORK_UPDATE.starts_with("install-login-service.ps1 "));
    assert!(WINDOWS_NETWORK_UPDATE.ends_with("-NetworkFile BUNDLE/gateway-network.json"));

    let state = LINUX_SCRIPT
        .lines()
        .find_map(|line| line.strip_prefix("state="))
        .expect("the script names its gateway state directory");
    let words: Vec<&str> = LINUX_NETWORK_UPDATE.split_whitespace().collect();
    assert_eq!(words[0], "install-login-service.sh");
    assert_eq!(words[1], "install");
    // IMAGE SHA256 VERSION POLICY MACHINE_IDENTITY NETWORK: six arguments.
    assert_eq!(words.len(), 8, "{LINUX_NETWORK_UPDATE}");
    assert_eq!(words[6], format!("{state}/machine-key.json"));
    assert_eq!(words[7], "BUNDLE/gateway-network.json");
    assert!(LINUX_SCRIPT.contains(
        "IMAGE EXPECTED_SHA256 VERSION POLICY_JSON [MACHINE_IDENTITY_JSON] [GATEWAY_NETWORK_JSON]"
    ));
}
