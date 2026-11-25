//! BK0 (plan 2026-11-14 „Bildkette“): `se screen bench --source` in the
//! worker: the test picture without it, a monitor the host does not list is
//! refused with `source_not_found`, and both directions of an older daemon.
use super::*;
use crate::screen::core::viewer_client::ViewerError;

fn monitor(id: &str) -> MonitorInfo {
    MonitorInfo {
        id: id.into(),
        name: String::new(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        primary: id == "0",
        is_virtual: false,
    }
}

#[test]
fn bildkette_task_bench_source_refuses_a_missing_monitor_with_code() {
    assert_eq!(bench_source(None).unwrap(), SourceId::Synthetic);
    let named = bench_source(Some("monitor:9")).unwrap();
    assert_eq!(named, SourceId::Monitor("9".into()));
    let monitors = [monitor("0"), monitor("1")];
    assert!(monitor_missing(&named, &monitors));
    assert!(!monitor_missing(&SourceId::Monitor("1".into()), &monitors));
    // The test picture and windows are never „missing“ here: the host
    // answers a window it does not have with its own code.
    assert!(!monitor_missing(&SourceId::Synthetic, &monitors));
    assert!(!monitor_missing(&SourceId::Window("42".into()), &monitors));
    // The refusal carries the code the CLI prints first (`code: text`).
    let refused = ViewerError::Refused(BENCH_SOURCE_NOT_FOUND.into());
    assert_eq!(refused.code(), "source_not_found");
    // A misspelt source is refused before any connection.
    let error = bench_source(Some("screen 2")).unwrap_err();
    assert!(error.starts_with("invalid source:"), "{error}");
}

#[test]
fn bildkette_task_bench_source_old_daemon_and_old_client() {
    let request = BenchRequest {
        codec: "tiles".into(),
        size: FrameSize::new(1280, 720),
        fps: 30,
        seconds: 10,
        source: None,
        delivery: None,
    };
    // Without `--source` the request is byte for byte the old one.
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 4, "{json}");
    // A client before BK0 sends no `source`: the test picture.
    let read: BenchRequest = serde_json::from_value(json).unwrap();
    assert_eq!(read.source, None);
    // A daemon before BK0 reads a request with `source` and ignores it.
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct OldBenchRequest {
        codec: String,
        size: FrameSize,
        fps: u16,
        seconds: u32,
    }
    let named = BenchRequest {
        source: Some("monitor:0".into()),
        ..request
    };
    let old: OldBenchRequest =
        serde_json::from_value(serde_json::to_value(&named).unwrap()).unwrap();
    assert_eq!(old.seconds, 10);
}
