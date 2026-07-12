use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::support::{assert_exit_code, assert_success, run, run_bounded, stdout, Sandbox};

#[test]
fn version_reports_the_package_version_without_gui_state() {
    let sandbox = Sandbox::new("version");
    let output = run(sandbox.command().arg("--version"));
    assert_success(&output);
    assert_eq!(
        stdout(&output).trim(),
        format!("se {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn shell_completion_setup_is_generated_without_gui_state() {
    let sandbox = Sandbox::new("completions");
    for shell in ["bash", "powershell"] {
        let output = run(sandbox.command().args(["completions", shell]));
        assert_success(&output);
        assert!(output.stderr.is_empty());
        let script = stdout(&output);
        assert!(script.contains("COMPLETE"));
        assert!(script.contains("se"));
    }
}

#[test]
fn doctor_json_reports_the_executable_and_isolated_state_paths() {
    let sandbox = Sandbox::new("doctor");
    let output = run(sandbox.command().args(["doctor", "--json"]));
    assert_success(&output);
    assert!(output.stderr.is_empty());

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output must be JSON");
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    let executable = report["executable"]
        .as_str()
        .map(PathBuf::from)
        .expect("doctor executable must be a path string");
    assert!(executable.is_absolute());
    assert!(executable.is_file());
    assert_eq!(
        PathBuf::from(
            report["app_data"]
                .as_str()
                .expect("doctor app_data must be a path string")
        ),
        sandbox.app_data_path("")
    );
    assert!(report["credential_backend"]
        .as_str()
        .is_some_and(|backend| !backend.trim().is_empty()));
    assert!(report.get("connections").is_some());
    assert!(report.get("daemon").is_some());
}

#[test]
fn doctor_returns_failure_for_corrupt_persistent_state() {
    let sandbox = Sandbox::new("doctor-corrupt-state");
    fs::create_dir_all(sandbox.app_data_path(""))
        .expect("create isolated application data directory");
    fs::write(
        sandbox.app_data_path("connections.txt"),
        b"not-a-valid-connection\n",
    )
    .expect("write corrupt connection metadata");

    let output = run(sandbox.command().args(["doctor", "--json"]));
    assert_exit_code(&output, 1);
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor failure output must remain JSON");
    assert_eq!(report["connections"]["state"], "error");
}

#[test]
fn sync_daemon_text_in_a_normal_argument_is_not_hijacked() {
    let sandbox = Sandbox::new("sync-daemon-argument");
    fs::write(sandbox.path("--sync-daemon"), b"ordinary file")
        .expect("create option-shaped fixture");

    let bounded = run_bounded(
        sandbox.command().arg("stat").arg("--").arg("--sync-daemon"),
        Duration::from_secs(3),
    );

    assert!(
        !bounded.timed_out,
        "se treated a positional --sync-daemon value as daemon mode"
    );
    assert_success(&bounded.output);
    assert!(stdout(&bounded.output).contains("name\t--sync-daemon\n"));
}
