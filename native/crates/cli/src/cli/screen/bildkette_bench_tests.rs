//! BK0 (plan 2026-11-14 „Bildkette“): `se screen bench --source` on the
//! command line and the new text lines of `bench` and `sessions`.
use clap::Parser;

use super::BenchArgs;
use crate::screen::core::frame_timing::{ChainReport, HostStations, StationUs};

#[derive(Parser)]
struct Wrapper {
    #[command(flatten)]
    args: BenchArgs,
}

fn parse(args: &[&str]) -> Result<BenchArgs, clap::Error> {
    let mut line = vec!["bench"];
    line.extend_from_slice(args);
    Wrapper::try_parse_from(line).map(|wrapper| wrapper.args)
}

#[test]
fn bildkette_task_bench_cli_source_is_optional_and_checked() {
    // Without `--source` the bench measures the test picture as before.
    assert_eq!(parse(&["laptop"]).unwrap().source(), Ok(None));
    assert_eq!(
        parse(&["laptop", "--source", "monitor:1"])
            .unwrap()
            .source(),
        Ok(Some("monitor:1".to_string()))
    );
    assert_eq!(
        parse(&["laptop", "--source", "window:4242"])
            .unwrap()
            .source(),
        Ok(Some("window:4242".to_string()))
    );
    // The test picture is no source of the host's screen.
    let synthetic = parse(&["laptop", "--source", "synthetic"]).unwrap();
    assert!(synthetic.source().is_err());
    // A misspelt source is refused before the device is contacted.
    let misspelt = parse(&["laptop", "--source", "screen2"]).unwrap();
    assert!(misspelt
        .source()
        .unwrap_err()
        .starts_with("invalid source:"));
    // `--local` measures this device only: no host source.
    assert!(parse(&["--local", "--source", "monitor:0"]).is_err());
}

#[test]
fn bildkette_task_bench_cli_text_names_host_timing_and_chain() {
    let chain = ChainReport {
        capture_path: Some("desktop_duplication".into()),
        convert: Some("gpu".into()),
        fps_limit: Some("encode_time".into()),
        ..ChainReport::default()
    };
    let text = super::super::output::chain_text(&chain);
    assert!(text.starts_with("capture=desktop_duplication\tfallback=-\tconvert=gpu"));
    assert!(text.contains("fps_limit=encode_time"), "{text}");
    assert!(text.contains("cursor_embedded=-"), "{text}");

    let mut session: crate::screen::core::registry::SessionView =
        serde_json::from_value(serde_json::json!({
            "id": "7", "device_id": "d", "device_name": "Laptop", "relation": "direct",
            "mode": "view", "source": "monitor:0", "path": "direct", "started_at": 0,
            "bitrate_kbps": 0, "fps": 30, "controlling": false, "state": "streaming"
        }))
        .unwrap();
    let before = super::super::output::sessions_text(std::slice::from_ref(&session));
    assert!(!before.contains("session_chain"), "{before}");
    session.chain = Some(chain);
    session.stations = Some(HostStations {
        capture: Some(StationUs {
            p50_us: 900,
            p95_us: 1_400,
        }),
        ..HostStations::default()
    });
    let text = super::super::output::sessions_text(&[session]);
    assert!(
        text.contains("session_chain\t7\tcapture=desktop_duplication"),
        "{text}"
    );
    assert!(
        text.contains("session_stage_us\t7\tcapture\tp50=900\tp95=1400"),
        "{text}"
    );
}
