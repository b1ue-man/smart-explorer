use std::time::Duration;

use crate::support::{assert_success, run_bounded, stderr, Sandbox, WorkerStopGuard};

const TEST_NAMESPACE: &str = "standalone_cli";

#[test]
fn windows_terminal_identity_and_worker_status_are_standalone() {
    let sandbox = Sandbox::new("share-windows-standalone");
    let _daemon = WorkerStopGuard { sandbox: &sandbox };

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

    let mut inbox_command = sandbox.command();
    inbox_command
        .env("SMART_EXPLORER_E2E_TEST_NAMESPACE", TEST_NAMESPACE)
        .args(["share", "request", "--json"]);
    let inbox = run_bounded(&mut inbox_command, Duration::from_secs(30));
    assert!(!inbox.timed_out, "standalone Share inbox timed out");
    assert_success(&inbox.output);
    let inbox: serde_json::Value =
        serde_json::from_slice(&inbox.output.stdout).expect("Share inbox must be JSON");
    assert_eq!(inbox["count"], 0);

    let completions = run_bounded(
        sandbox.command().args(["completions", "powershell"]),
        Duration::from_secs(10),
    );
    assert!(
        !completions.timed_out,
        "PowerShell completion generation timed out"
    );
    assert_success(&completions.output);
    assert!(String::from_utf8_lossy(&completions.output.stdout).contains("COMPLETE"));
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
