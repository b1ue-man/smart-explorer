//! Task tests of WP3 (design 10.3, U2 and U3) for the CLI and MCP: `se rdp
//! open` needs only the device and still accepts the switches of 0.5.188,
//! the MCP tool `rdp` opens with only `device`, and TCP starts need neither
//! an acknowledgement nor a lifetime. Included by `cli/mcp/rdp_tool.rs`.
//! Prefix `einrichtung_task_`.
use super::*;
use crate::cli::rdp::{OpenArgs, RdpArgs};
use crate::cli::tcp::StartArgs;
use crate::rdp::OpenOptions;
use clap::{CommandFactory, Parser};

/// `se` with only its `rdp` command, as `cli::Command::Rdp` nests it.
#[derive(Parser)]
#[command(name = "se")]
#[allow(dead_code)]
struct SeRoot {
    #[command(subcommand)]
    command: SeCommand,
}

#[derive(clap::Subcommand)]
#[allow(dead_code)]
enum SeCommand {
    Rdp(RdpArgs),
}

#[derive(Parser)]
#[command(name = "open")]
struct OpenRoot {
    #[command(flatten)]
    open: OpenArgs,
}

#[derive(Parser)]
#[command(name = "start")]
struct StartRoot {
    #[command(flatten)]
    start: StartArgs,
}

/// U2: the help of `se rdp open` names only the device, `--remote-app` and
/// `--json`; every switch of 0.5.188 still parses and is ignored.
#[test]
fn einrichtung_task_cli_rdp_open_needs_only_the_device() {
    let mut root = SeRoot::command();
    let open = root
        .find_subcommand_mut("rdp")
        .and_then(|rdp| rdp.find_subcommand_mut("open"))
        .expect("se rdp open");
    let help = open.render_help().to_string();
    for shown in ["<DEVICE>", "--remote-app", "--json"] {
        assert!(help.contains(shown), "{shown} in {help}");
    }
    for hidden in [
        "--service",
        "--client",
        "--certificate-policy",
        "--acknowledge",
        "--lifetime-secs",
        "--wait",
        "--notify-errors",
    ] {
        assert!(!help.contains(hidden), "{hidden} hidden in {help}");
    }
    let plain = OpenRoot::try_parse_from(["open", "laptop"]).unwrap().open;
    assert_eq!(plain.device, "laptop");
    assert!(plain.ignored().is_empty());
    let legacy = OpenRoot::try_parse_from([
        "open",
        "laptop",
        "--service",
        "termservice",
        "--client",
        "mstsc",
        "--certificate-policy",
        "native-dialog",
        "--acknowledge-local-access",
        "--acknowledge-session-effect",
        "--acknowledge-certificate-dialog",
        "--lifetime-secs",
        "900",
        "--wait",
        "30",
        "--remote-app",
        "calc",
    ])
    .unwrap()
    .open;
    assert_eq!(legacy.remote_app.as_deref(), Some("calc"));
    assert_eq!(
        legacy.ignored(),
        [
            "--certificate-policy",
            "--acknowledge-local-access",
            "--acknowledge-session-effect",
            "--acknowledge-certificate-dialog",
            "--lifetime-secs"
        ]
    );
    // `--service`, `--client` and an explicit `--wait` still override the
    // derived choice (design 2.9); nothing else changes the start.
    let options = legacy.options().unwrap();
    assert_eq!(options.service.as_deref(), Some("termservice"));
    assert_eq!(options.client.as_deref(), Some("mstsc"));
    assert_eq!(options.wait, Some(std::time::Duration::from_secs(30)));
    assert_eq!(options.remote_app.as_deref(), Some("calc"));
    assert!(options.credentials.is_none());
    let derived = plain.options().unwrap();
    assert!(derived.service.is_none() && derived.client.is_none() && derived.wait.is_none());
    assert!(SeRoot::try_parse_from(["se", "rdp", "open", "laptop", "--json"]).is_ok());
}

/// U3: the catalog publishes `rdp` with only `action` required, and `open`
/// with only `device` opens through the injected opener; the old arguments
/// are still accepted and change nothing.
#[test]
fn einrichtung_task_mcp_rdp_open_with_only_device() {
    let tool = super::super::catalog::tools()
        .into_iter()
        .find(|tool| tool.name == "rdp")
        .expect("the catalog lists the tool `rdp`");
    assert_eq!(tool.input_schema["required"], serde_json::json!(["action"]));
    assert_eq!(tool.input_schema["additionalProperties"], false);
    let properties = properties();
    for legacy in [
        "service",
        "client",
        "certificate_policy",
        "acknowledge_local_access",
        "acknowledge_session_effect",
        "acknowledge_certificate_dialog",
        "lifetime_secs",
        "wait",
    ] {
        // Still accepted (the schema is closed), none with a default.
        assert!(properties.get(legacy).is_some(), "{legacy} stays accepted");
        assert!(properties[legacy].get("default").is_none(), "{legacy}");
    }
    for call in [
        serde_json::json!({"action": "open", "device": "laptop"}),
        serde_json::json!({
            "action": "open",
            "device": "laptop",
            "acknowledge_local_access": false,
            "lifetime_secs": 900,
            "certificate_policy": "exact_peer_pin"
        }),
    ] {
        let map = call.as_object().unwrap().clone();
        let mut opened = Vec::new();
        let mut resolve = |selector: &str| {
            assert_eq!(selector, "laptop");
            Ok::<_, String>(("device-1".to_string(), "key-1".to_string()))
        };
        let mut open = |device: &str, key: &str, options: OpenOptions| {
            assert!(options.service.is_none() && options.client.is_none());
            assert!(options.wait.is_none(), "no default time limit");
            opened.push((device.to_string(), key.to_string(), options.remote_app));
            Ok::<_, String>(serde_json::json!({"process_id": 7}))
        };
        let result = rdp_with(
            &Args::new(Some(&map)),
            |_| Err("no probe without a device".to_string()),
            &mut resolve,
            &mut open,
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            opened,
            [("device-1".to_string(), "key-1".to_string(), None)]
        );
    }
    let map = serde_json::json!({"action": "open"})
        .as_object()
        .unwrap()
        .clone();
    let missing = rdp_with(
        &Args::new(Some(&map)),
        |_| Err("unused".to_string()),
        &mut |_: &str| Ok::<_, String>((String::new(), String::new())),
        &mut |_: &str, _: &str, _: OpenOptions| Ok::<_, String>(Value::Null),
    );
    assert!(missing.is_err(), "open names its device");
}

/// TCP: `se tcp start` and the MCP tool start without an acknowledgement
/// and without a fixed lifetime; an explicit lifetime stays possible.
#[test]
fn einrichtung_task_cli_tcp_start_without_acknowledgement_or_lifetime() {
    let start = StartRoot::try_parse_from(["start", "laptop", "--target-port", "5432"])
        .unwrap()
        .start;
    let request = start.request().unwrap();
    assert!(request.acknowledge_local_access);
    assert_eq!(request.lifetime_secs, 0);
    let legacy = StartRoot::try_parse_from([
        "start",
        "laptop",
        "--target-port",
        "80",
        "--lifetime-secs",
        "900",
        "--acknowledge-local-access",
    ])
    .unwrap()
    .start;
    assert_eq!(legacy.request().unwrap().lifetime_secs, 900);
    let help = StartRoot::command().render_help().to_string();
    assert!(!help.contains("--acknowledge-local-access"), "{help}");
    let map = serde_json::json!({"action": "start", "device": "laptop", "target_port": 5432})
        .as_object()
        .unwrap()
        .clone();
    let request = super::super::tcp_tool::start_request(&Args::new(Some(&map))).unwrap();
    assert!(request.acknowledge_local_access);
    assert_eq!(request.lifetime_secs, 0);
    assert!(super::super::tcp_tool::properties()["lifetime_secs"]
        .get("default")
        .is_none());
}

/// `se tcp services` / MCP `services`: the device's answer with the address
/// this device bound for each port.
#[test]
fn einrichtung_task_cli_tcp_services_show_the_address_here() {
    use crate::desktop::core::tiles::{TileFeed, TileService};
    use crate::share::loopback_services::{BindScope, LoopbackService, OwnerClass};
    let service = LoopbackService {
        port: 8080,
        scope: BindScope::LoopbackOnly,
        owner: OwnerClass::System,
        process: Some("python3".into()),
        pid: Some(11),
    };
    let address: std::net::SocketAddr = "127.9.8.7:8080".parse().unwrap();
    let feed = TileFeed {
        known: true,
        services: vec![TileService {
            port: 8080,
            process: Some("python3".into()),
            address: Some(address),
            url: Some("http://127.9.8.7:8080".into()),
        }],
        control: true,
        ..TileFeed::default()
    };
    let value = crate::cli::tcp::services_value("Laptop", &[service], None, Some(&feed));
    assert_eq!(value["device"], "Laptop");
    assert_eq!(value["services"][0]["port"], 8080);
    assert_eq!(value["services"][0]["address"], "127.9.8.7:8080");
    assert_eq!(value["services"][0]["url"], "http://127.9.8.7:8080");
    assert!(value["unavailable"].is_null());
}
