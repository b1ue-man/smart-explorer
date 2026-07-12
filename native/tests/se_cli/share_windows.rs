use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use crate::support::{assert_success, run_bounded, stderr, Sandbox};

const TEST_NAMESPACE: &str = "standalone_cli";

#[test]
fn windows_terminal_identity_and_worker_status_are_standalone() {
    let sandbox = Sandbox::new("share-windows-standalone");

    let first = identity(&sandbox);
    let second = identity(&sandbox);
    assert_eq!(first["device_id"], second["device_id"]);
    assert_eq!(first["node_id"], second["node_id"]);
    assert_eq!(first["direct_code"], second["direct_code"]);

    let mut command = sandbox.command();
    command
        .env("SMART_EXPLORER_E2E_TEST_NAMESPACE", TEST_NAMESPACE)
        .args(["share", "status", "--json"]);
    let status = run_bounded(&mut command, Duration::from_secs(30));
    assert!(!status.timed_out, "standalone Share status timed out");
    assert_success(&status.output);
    assert!(
        !stderr(&status.output).contains("wurde nach dem Neustart nicht bereit"),
        "standalone status reported a false restart-readiness failure"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&status.output.stdout).expect("Share status must be JSON");
    assert!(value["running"].is_boolean());
    assert!(value["connected"].is_boolean());

    stop_daemon(&sandbox);
}

fn identity(sandbox: &Sandbox) -> serde_json::Value {
    let mut command = sandbox.command();
    command
        .env("SMART_EXPLORER_E2E_TEST_NAMESPACE", TEST_NAMESPACE)
        .args(["share", "identity", "--json"]);
    let result = run_bounded(&mut command, Duration::from_secs(20));
    assert!(!result.timed_out, "standalone Share identity timed out");
    assert_success(&result.output);
    assert!(result.output.stderr.is_empty());
    serde_json::from_slice(&result.output.stdout).expect("Share identity must be JSON")
}

fn stop_daemon(sandbox: &Sandbox) {
    let stop = sandbox.app_data_path("sync/daemon.stop");
    fs::create_dir_all(stop.parent().expect("daemon stop parent"))
        .expect("create daemon control directory");
    fs::write(&stop, b"stop").expect("write daemon stop marker");
    let heartbeat = sandbox.app_data_path("sync/daemon.heartbeat");
    let deadline = Instant::now() + Duration::from_secs(10);
    while heartbeat.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
}
