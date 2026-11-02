//! Acceptance of `se screen` (SCR-M2): the JSON shapes of every command,
//! the text lines, the device selection and the command line.
use clap::Parser;
use serde_json::Value;

use super::bench::{parse_size, SizeArg};
use super::{device_choices, output, select_from, ScreenCommand};
use crate::daemon::ScreenProbeReport;
use crate::screen::core::bench::{BenchReport, BitrateKbps, LatencyMs};
use crate::screen::core::input_model::InputCapabilities;
use crate::screen::core::negotiate::CodecOffer;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::registry::SessionView;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_hello::{HostPlatform, SessionMode};
use crate::share::access_grant::CapabilityDecision;
use crate::share::{DirectContact, ShareProfiles};

#[derive(Parser)]
struct Wrapper {
    #[command(subcommand)]
    command: ScreenCommand,
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort_unstable();
    keys
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    names.sort_unstable();
    names
}

fn contact(id: &str, name: &str, fingerprint: &str) -> DirectContact {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "display_name": name,
        "lookup_id": format!("lookup-{id}"),
        "expected_fingerprint": fingerprint,
        "remote_device_id": format!("device-{id}"),
        "auto_connect": false,
        "auto_open": false,
    }))
    .unwrap()
}

fn probe() -> ScreenProbeReport {
    ScreenProbeReport {
        device: "Share Direkt: Laptop".into(),
        supported: true,
        platform: Some(HostPlatform::Linux),
        view: CapabilityDecision::Ask,
        control: CapabilityDecision::Off,
        monitors: Vec::new(),
        input: InputCapabilities::default(),
        codecs: vec![CodecOffer::tiles()],
        limits: vec!["test_picture_only".into()],
    }
}

fn bench() -> BenchReport {
    BenchReport {
        codec: "tiles".into(),
        encoder: "tiles".into(),
        decoder: "tiles".into(),
        path: "direct".into(),
        frames_sent: 300,
        frames_shown: 299,
        fps_mean: 29.9,
        fps_min: 28.0,
        latency_ms: LatencyMs {
            p50: 12.0,
            p95: 30.5,
            max: 41.0,
        },
        bitrate_kbps: BitrateKbps {
            mean: 900.0,
            max: 4100.0,
        },
        keyframes: 2,
        recoveries: vec![250.0],
        corrupt_frames: 0,
        static_psnr_db: None,
    }
}

fn session() -> SessionView {
    SessionView {
        id: "1".into(),
        device_id: "device-c1".into(),
        device_name: "Laptop".into(),
        relation: "direct".into(),
        room_id: None,
        mode: SessionMode::View,
        source: "synthetic".into(),
        codec: Some("tiles".into()),
        encoder: Some("tiles".into()),
        path: "relay".into(),
        started_at: 1_760_000_000,
        bitrate_kbps: 900,
        fps: 30,
        controlling: false,
        state: SessionState::WaitingForApproval,
    }
}

#[test]
fn screen_task_cli_json_shapes() {
    let report = serde_json::to_value(probe()).unwrap();
    let expected = [
        "device",
        "supported",
        "platform",
        "view",
        "control",
        "monitors",
        "input",
        "codecs",
        "limits",
    ];
    assert_eq!(keys(&report), sorted(&expected));
    assert_eq!(report["view"], "ask");
    assert_eq!(report["platform"], "linux");
    assert_eq!(report["codecs"][0]["codec"], "tiles");

    let measured = serde_json::to_value(bench()).unwrap();
    let expected = [
        "codec",
        "encoder",
        "decoder",
        "path",
        "frames_sent",
        "frames_shown",
        "fps_mean",
        "fps_min",
        "latency_ms",
        "bitrate_kbps",
        "keyframes",
        "recoveries",
        "corrupt_frames",
        "static_psnr_db",
    ];
    assert_eq!(keys(&measured), sorted(&expected));
    assert_eq!(
        keys(&measured["latency_ms"]),
        sorted(&["p50", "p95", "max"])
    );
    assert_eq!(keys(&measured["bitrate_kbps"]), sorted(&["mean", "max"]));
    // Infinite PSNR (identical block) is JSON null, a number otherwise.
    assert_eq!(measured["static_psnr_db"], Value::Null);
    let mut lossy = bench();
    lossy.static_psnr_db = Some(41.5);
    assert_eq!(serde_json::to_value(lossy).unwrap()["static_psnr_db"], 41.5);

    let sessions = output::sessions_json(&[session()]);
    assert_eq!(keys(&sessions), ["sessions"]);
    assert_eq!(sessions["sessions"][0]["state"], "waiting_for_approval");
    assert_eq!(keys(&sessions["sessions"][0]).len(), 15);
    assert_eq!(output::stop_json(2), serde_json::json!({ "stopped": 2 }));

    let profiles = ShareProfiles {
        direct_contacts: vec![contact("c1", "Laptop", "fp-laptop")],
        ..ShareProfiles::default()
    };
    let choices = device_choices(&profiles, 1_760_000_000);
    let devices = output::devices_json(&choices, &[Some(Ok(probe()))]);
    let device = &devices["devices"][0];
    let expected = [
        "target",
        "device_id",
        "name",
        "relation",
        "room",
        "online",
        "probe",
    ];
    assert_eq!(keys(device), sorted(&expected));
    assert_eq!(device["target"], "share://direct/c1");
    assert_eq!(device["online"], false);
    assert_eq!(device["probe"]["view"], "ask");

    // Text lines.
    let text = output::probe_text(&probe());
    assert!(
        text.starts_with("screen\tShare Direkt: Laptop\tsupported=yes\n"),
        "{text}"
    );
    assert!(
        text.contains("view\task\ncontrol\toff\ncodecs\ttiles\n"),
        "{text}"
    );
    let text = output::bench_text(&bench());
    assert!(text.contains("frames\tsent=300\tshown=299\tcorrupt=0\tkeyframes=2\n"));
    assert!(text.ends_with("static_psnr_db\tinf\n"), "{text}");
    let text = output::sessions_text(&[session()]);
    assert!(text.starts_with("sessions\t1\nsession\t1\tLaptop\tdirect\t-\tview\t"));
    let text = output::devices_text(&choices, &[None]);
    assert_eq!(
        text,
        "devices\t1\ndevice\tshare://direct/c1\tLaptop\tdirect\t-\toffline\n"
    );
}

#[test]
fn screen_task_cli_device_selection_and_options() {
    let profiles = ShareProfiles {
        direct_contacts: vec![
            contact("c1", "Laptop", "fp-laptop-1234"),
            contact("c2", "Lab PC", "fp-lab-5678"),
        ],
        ..ShareProfiles::default()
    };
    let choices = device_choices(&profiles, 1_760_000_000);
    let by_endpoint = select_from(&choices, "share://direct/c2").unwrap();
    assert_eq!(by_endpoint.name, "Lab PC");
    assert_eq!(select_from(&choices, "laptop").unwrap().name, "Laptop");
    assert_eq!(select_from(&choices, "fp-lab").unwrap().name, "Lab PC");
    let ambiguous = select_from(&choices, "fp-la").unwrap_err();
    assert!(ambiguous.contains("ambiguous"), "{ambiguous}");
    assert!(select_from(&choices, "Desk").is_err());
    assert!(select_from(&choices, "share://direct/c1/Docs").is_err());

    assert_eq!(
        parse_size("1920x1080").unwrap(),
        SizeArg(FrameSize::new(1920, 1080))
    );
    assert!(parse_size("1920").is_err());
    let parsed = Wrapper::try_parse_from(["screen", "bench", "--local", "--size", "640x360"]);
    assert!(matches!(parsed.unwrap().command, ScreenCommand::Bench(_)));
    assert!(
        Wrapper::try_parse_from(["screen", "bench"]).is_err(),
        "device or --local"
    );
    assert!(Wrapper::try_parse_from(["screen", "bench", "Laptop", "--local"]).is_err());
    assert!(
        Wrapper::try_parse_from(["screen", "stop"]).is_err(),
        "id or --all"
    );
    let stop = Wrapper::try_parse_from(["screen", "stop", "--all", "--json"]).unwrap();
    assert_eq!(
        stop.command,
        ScreenCommand::Stop {
            session: None,
            all: true,
            json: true
        }
    );
}
