use std::ffi::OsString;
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::support::{
    assert_success, collect_bounded, run_bounded, spawn_captured, stderr, Sandbox,
};

#[path = "share/mock_signal.rs"]
mod mock_signal;

const CONCURRENT_FIRST_RUNS: usize = 16;
const CONCURRENT_WORKER_STARTS: usize = 8;
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(15);

struct PublicIdentity {
    device_id: String,
    device_name: String,
    fingerprint: String,
    node_id: String,
    direct_code: String,
}

#[test]
fn headless_identity_is_created_once_and_reloads_stably() {
    let sandbox = Sandbox::new("share-identity-headless");

    let first = run_identity(&sandbox, false);
    let first_identity = parse_identity(&first);
    assert!(first.stderr.is_empty(), "clean identity wrote to stderr");
    assert_direct_code_shape(&first_identity);
    assert_exact_identity_secret_records(&sandbox);

    let second = run_identity(&sandbox, false);
    let second_identity = parse_identity(&second);
    assert!(second.stderr.is_empty(), "identity reload wrote to stderr");
    assert_same_identity(&first_identity, &second_identity);
    assert_exact_identity_secret_records(&sandbox);
}

#[test]
fn healthy_identity_repair_is_refused_before_starting_a_worker() {
    let sandbox = Sandbox::new("share-identity-healthy-repair");
    let original = parse_identity(&run_identity(&sandbox, false));

    let repair = run_identity(&sandbox, true);
    assert_eq!(repair.status.code(), Some(1));
    assert!(repair.stdout.is_empty());
    assert!(stderr(&repair).contains("vollstaendig"));
    assert!(
        !sandbox.app_data_path("sync/daemon.token").exists(),
        "healthy repair unexpectedly started the background worker"
    );

    let reloaded = parse_identity(&run_identity(&sandbox, false));
    assert_same_identity(&original, &reloaded);
    assert_exact_identity_secret_records(&sandbox);
}

#[test]
fn share_status_coordinates_with_a_starting_daemon_singleton() {
    let sandbox = Sandbox::new("share-worker-starting-singleton");
    let _daemon = DaemonStopGuard { sandbox: &sandbox };
    let _identity = parse_identity(&run_identity(&sandbox, false));
    let runtime = sandbox.path("runtime");
    fs::create_dir(&runtime).expect("create isolated daemon runtime directory");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .expect("secure isolated daemon runtime directory");

    let lock_path = runtime.join("smart-explorer-sync-daemon.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("create daemon singleton fixture");
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))
        .expect("secure daemon singleton fixture");
    assert_eq!(
        unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "acquire daemon singleton fixture"
    );

    let stop = sandbox.app_data_path("sync/daemon.stop");
    let release = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !stop.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let stop_observed = stop.exists();
        if stop_observed {
            thread::sleep(Duration::from_millis(300));
        }
        drop(lock_file);
        stop_observed
    });

    let mut status_command = sandbox.command();
    status_command
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("SE_SHARE_RELAY_URL", "disabled")
        .args(["share", "status", "--json"]);
    let status = run_bounded(&mut status_command, Duration::from_secs(15));
    assert!(
        !status.timed_out,
        "Share status timed out during daemon handoff"
    );
    assert_success(&status.output);
    assert!(
        !stderr(&status.output).contains("wurde nach dem Neustart nicht bereit"),
        "Share status reported the generic restart-readiness failure"
    );
    serde_json::from_slice::<serde_json::Value>(&status.output.stdout)
        .expect("Share status stdout must be JSON");
    assert!(
        release.join().expect("join daemon singleton fixture"),
        "Share client did not request a coordinated daemon handoff"
    );

    let contender = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .expect("open daemon singleton after handoff");
    let contender_result =
        unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    let contender_error = std::io::Error::last_os_error();
    assert!(
        contender_result == -1
            && contender_error
                .raw_os_error()
                .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN),
        "no replacement daemon retained the singleton after a successful status: {contender_error}"
    );
}

#[test]
fn share_status_waits_for_identity_initialization_after_ping_is_published() {
    let sandbox = Sandbox::new("share-worker-slow-identity-init");
    let _daemon = DaemonStopGuard { sandbox: &sandbox };
    let runtime = sandbox.path("runtime");
    fs::create_dir(&runtime).expect("create isolated daemon runtime directory");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .expect("secure isolated daemon runtime directory");

    let lock_directory = sandbox.app_data_path("identity-lock-v1");
    fs::create_dir_all(&lock_directory).expect("create identity lock directory");
    fs::set_permissions(&lock_directory, fs::Permissions::from_mode(0o700))
        .expect("secure identity lock directory");
    let lock_path = lock_directory.join("transaction.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("create identity lock fixture");
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))
        .expect("secure identity lock fixture");
    assert_eq!(
        unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "acquire identity lock fixture"
    );

    let ipc = sandbox.app_data_path("sync/daemon.ipc");
    let release = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !ipc.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let control_plane_published = ipc.exists();
        if control_plane_published {
            thread::sleep(Duration::from_millis(1_200));
        }
        drop(lock_file);
        control_plane_published
    });

    let mut status_command = sandbox.command();
    status_command
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("SE_SHARE_RELAY_URL", "disabled")
        .args(["share", "status", "--json"]);
    let status = run_bounded(&mut status_command, Duration::from_secs(15));
    assert!(
        !status.timed_out,
        "Share status timed out during identity initialization"
    );
    assert_success(&status.output);
    serde_json::from_slice::<serde_json::Value>(&status.output.stdout)
        .expect("Share status stdout must be JSON");
    assert!(
        release.join().expect("join identity lock fixture"),
        "daemon did not publish authenticated Ping before identity loading"
    );
}

#[test]
fn concurrent_fresh_share_status_commands_converge_on_one_worker() {
    let sandbox = Sandbox::new("share-worker-concurrent-start");
    let _daemon = DaemonStopGuard { sandbox: &sandbox };
    let _identity = parse_identity(&run_identity(&sandbox, false));
    let runtime = sandbox.path("runtime");
    fs::create_dir(&runtime).expect("create isolated daemon runtime directory");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .expect("secure isolated daemon runtime directory");

    let mut children = Vec::with_capacity(CONCURRENT_WORKER_STARTS);
    for _ in 0..CONCURRENT_WORKER_STARTS {
        let mut command = sandbox.command();
        command
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("SE_SHARE_RELAY_URL", "disabled")
            .args(["share", "status", "--json"]);
        children.push(spawn_captured(&mut command));
    }

    for child in children {
        let status = collect_bounded(child, Duration::from_secs(35));
        assert!(!status.timed_out, "concurrent Share status timed out");
        assert_success(&status.output);
        serde_json::from_slice::<serde_json::Value>(&status.output.stdout)
            .expect("concurrent Share status stdout must be JSON");
        assert!(
            !stderr(&status.output).contains("wurde nach dem Neustart nicht bereit"),
            "concurrent Share status rejected the healthy winning worker"
        );
    }

    // One desktop worker is two `--sync-daemon` processes: the guardian and
    // the child it supervises. Every further one is a start that did not end.
    let deadline = Instant::now() + Duration::from_secs(8);
    let daemons = loop {
        let daemons = sandbox_daemons(&sandbox);
        if daemons.is_one_supervised_worker() || Instant::now() >= deadline {
            break daemons;
        }
        thread::sleep(Duration::from_millis(50));
    };
    assert!(
        daemons.is_one_supervised_worker(),
        "concurrent starts left detached singleton waiters: {daemons:?}"
    );
}

#[test]
fn share_status_supersedes_a_stale_handoff_marker() {
    let sandbox = Sandbox::new("share-worker-stale-handoff");
    let _daemon = DaemonStopGuard { sandbox: &sandbox };
    let _identity = parse_identity(&run_identity(&sandbox, false));
    let runtime = sandbox.path("runtime");
    fs::create_dir(&runtime).expect("create isolated daemon runtime directory");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .expect("secure isolated daemon runtime directory");

    let sync = sandbox.app_data_path("sync");
    fs::create_dir_all(&sync).expect("create daemon control directory");
    fs::set_permissions(&sync, fs::Permissions::from_mode(0o700))
        .expect("secure daemon control directory");
    fs::write(
        sync.join("daemon.stop"),
        b"handoff:00000000000000000000000000000000",
    )
    .expect("write stale handoff marker");

    let mut status_command = sandbox.command();
    status_command
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("SE_SHARE_RELAY_URL", "disabled")
        .args(["share", "status", "--json"]);
    let status = run_bounded(&mut status_command, Duration::from_secs(15));
    assert!(
        !status.timed_out,
        "Share status timed out after stale handoff"
    );
    assert_success(&status.output);
    serde_json::from_slice::<serde_json::Value>(&status.output.stdout)
        .expect("Share status stdout must be JSON");
}

#[test]
fn active_standalone_worker_restarts_with_the_repaired_identity() {
    let sandbox = Sandbox::new("share-identity-live-worker-repair");
    let _daemon = DaemonStopGuard { sandbox: &sandbox };
    let signal = mock_signal::SignalServer::start();
    let original = parse_identity(&run_identity(&sandbox, false));
    let runtime = sandbox.path("runtime");
    fs::create_dir(&runtime).expect("create isolated daemon runtime directory");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .expect("secure isolated daemon runtime directory");
    let mut configure_command = sandbox.command();
    configure_command
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("SE_SHARE_RELAY_URL", "disabled")
        .args(["share", "configure", "--server", &signal.endpoint()]);

    let configured = run_bounded(&mut configure_command, Duration::from_secs(20));
    assert!(!configured.timed_out, "share configure timed out");
    assert_success(&configured.output);
    assert!(share_worker_running(&sandbox));

    fs::remove_file(secret_record_path(&sandbox, "share:identity:iroh_secret"))
        .expect("remove persisted Iroh secret to exercise live repair");

    let repaired = run_bounded(
        sandbox
            .command()
            .args(["share", "identity", "--json", "--repair"]),
        Duration::from_secs(20),
    );
    assert!(!repaired.timed_out, "live identity repair timed out");
    assert_success(&repaired.output);
    let replacement = parse_identity(&repaired.output);
    assert_ne!(replacement.device_id, original.device_id);
    assert_ne!(replacement.node_id, original.node_id);
    assert!(stderr(&repaired.output).contains("re-pair"));
    assert!(share_worker_running(&sandbox));
    assert_exact_identity_secret_records(&sandbox);

    let stopped = run_bounded(
        sandbox.command().args(["share", "worker", "stop"]),
        Duration::from_secs(10),
    );
    assert!(!stopped.timed_out, "Share worker stop timed out");
    assert_success(&stopped.output);
}

/// A device paired with anything keeps an identity whose key left the secure
/// store until the user repairs it, because every peer must confirm the new
/// one (`share/os/shared/identity_store.rs`, `load_or_replace_unpaired`).
#[test]
fn legacy_identity_requires_explicit_repair_and_reloads_the_replacement() {
    let sandbox = Sandbox::new("share-identity-legacy-repair");
    let legacy = write_legacy_identity(&sandbox);
    write_profile_with_saved_room(&sandbox);

    let configure = sandbox
        .command()
        .args(["share", "configure", "--server", "silasweis.de:51820"])
        .output()
        .expect("launch standalone se share configure command");
    assert_eq!(configure.status.code(), Some(1));
    assert!(
        configure.stdout.is_empty(),
        "failed configure wrote to stdout"
    );
    assert!(stderr(&configure).contains("se share identity --repair"));
    assert!(!sandbox.app_data_path("share_server.txt").exists());

    let failed = run_identity(&sandbox, false);
    assert_eq!(
        failed.status.code(),
        Some(1),
        "legacy identity without secrets must fail"
    );
    assert!(failed.stdout.is_empty(), "failed identity wrote to stdout");
    assert!(
        stderr(&failed).contains("se share identity --repair"),
        "missing-secret error did not provide the explicit repair command"
    );
    assert_eq!(
        fs::read(&legacy.metadata_path).expect("re-read legacy identity metadata"),
        legacy.metadata,
        "normal identity lookup rewrote legacy metadata"
    );
    assert_no_identity_secret_records(&sandbox);

    let repaired = run_identity(&sandbox, true);
    let repaired_identity = parse_identity(&repaired);
    assert!(
        stderr(&repaired).starts_with("warning: ")
            && stderr(&repaired).to_ascii_lowercase().contains("re-pair"),
        "identity replacement did not warn that peers require re-pairing"
    );
    assert_replaces_legacy_identity(&sandbox, &legacy, &repaired_identity);

    let reloaded = run_identity(&sandbox, false);
    let reloaded_identity = parse_identity(&reloaded);
    assert!(
        reloaded.stderr.is_empty(),
        "repaired identity reload warned"
    );
    assert_same_identity(&repaired_identity, &reloaded_identity);
    assert_exact_identity_secret_records(&sandbox);
}

/// Paired with nothing, the same identity is replaced by the ordinary lookup:
/// a new identity breaks no pairing, and Share starts at all.
#[test]
fn unpaired_legacy_identity_is_replaced_without_explicit_repair() {
    let sandbox = Sandbox::new("share-identity-legacy-unpaired");
    let legacy = write_legacy_identity(&sandbox);

    let replaced = parse_identity(&run_identity(&sandbox, false));
    assert_replaces_legacy_identity(&sandbox, &legacy, &replaced);

    let reloaded = run_identity(&sandbox, false);
    assert!(
        reloaded.stderr.is_empty(),
        "replaced identity reload warned"
    );
    assert_same_identity(&replaced, &parse_identity(&reloaded));
    assert_exact_identity_secret_records(&sandbox);
}

/// A v0.5.119 identity file whose secrets never reached the secure store.
struct LegacyIdentity {
    metadata_path: PathBuf,
    metadata: Vec<u8>,
    device_id: &'static str,
    device_name: &'static str,
    lookup_id: &'static str,
    node_id: String,
    fingerprint: String,
}

fn write_legacy_identity(sandbox: &Sandbox) -> LegacyIdentity {
    let metadata_path = sandbox.app_data_path("share_identity.json");
    fs::create_dir_all(
        metadata_path
            .parent()
            .expect("share identity metadata has a parent"),
    )
    .expect("create legacy identity directory");
    let device_id = "255f2f14-5633-4baa-b46b-a33cc78e43ce";
    let device_name = "Legacy Workstation";
    let lookup_id = "00112233445566778899aabb";
    let node_id = iroh::SecretKey::generate().public().to_string();
    let fingerprint = fingerprint(&node_id);
    let metadata = serde_json::to_vec_pretty(&serde_json::json!({
        "device_id": device_id,
        "device_name": device_name,
        "direct_lookup_id": lookup_id,
        "public_key": node_id,
        "fingerprint": fingerprint,
        "node_id": node_id,
    }))
    .expect("serialize v0.5.119 identity metadata");
    fs::write(&metadata_path, &metadata).expect("write legacy identity metadata");
    LegacyIdentity {
        metadata_path,
        metadata,
        device_id,
        device_name,
        lookup_id,
        node_id,
        fingerprint,
    }
}

/// One saved room, not joined automatically: the device counts as paired.
fn write_profile_with_saved_room(sandbox: &Sandbox) {
    let profile = serde_json::json!({
        "schema_version": 8,
        "rooms": [{
            "id": "legacy-room",
            "name": "Team",
            "room_id": "r1",
            "auto_join": false,
            "last_seen": null,
            "exports": {"roots": []},
        }],
    });
    fs::write(
        sandbox.app_data_path("share_profiles.json"),
        serde_json::to_vec_pretty(&profile).expect("serialize paired Share profile"),
    )
    .expect("write paired Share profile");
}

fn assert_replaces_legacy_identity(
    sandbox: &Sandbox,
    legacy: &LegacyIdentity,
    replacement: &PublicIdentity,
) {
    assert_eq!(replacement.device_name, legacy.device_name);
    assert!(
        replacement.device_id != legacy.device_id,
        "replacement retained the legacy device id"
    );
    assert!(
        replacement.node_id != legacy.node_id,
        "replacement retained the legacy Iroh node id"
    );
    assert!(
        replacement.fingerprint != legacy.fingerprint,
        "replacement retained the legacy fingerprint"
    );
    assert_direct_code_shape(replacement);

    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(&legacy.metadata_path).expect("read replaced identity metadata"),
    )
    .expect("replaced identity metadata must be JSON");
    assert!(
        json_string(&metadata, "direct_lookup_id") != legacy.lookup_id,
        "replacement retained the legacy direct lookup id"
    );
    assert!(
        json_string(&metadata, "public_key") != legacy.node_id,
        "replacement retained the legacy public key"
    );
    assert_eq!(json_string(&metadata, "device_name"), legacy.device_name);
    assert_exact_identity_secret_records(sandbox);
}

#[test]
fn concurrent_headless_first_run_converges_on_one_identity() {
    let sandbox = Sandbox::new("share-identity-concurrent");
    let gate = sandbox.path("identity-start-gate");
    let readiness = sandbox.path("identity-ready");
    fs::create_dir(&readiness).expect("create identity start readiness directory");

    let mut children = Vec::with_capacity(CONCURRENT_FIRST_RUNS);
    let mut ready_paths = Vec::with_capacity(CONCURRENT_FIRST_RUNS);
    for index in 0..CONCURRENT_FIRST_RUNS {
        let ready = readiness.join(index.to_string());
        let mut command = gated_identity_command(&sandbox, &ready, &gate);
        children.push(spawn_captured(&mut command));
        ready_paths.push(ready);
    }

    let all_ready = wait_for_readiness(&ready_paths, Duration::from_secs(10));
    fs::write(&gate, b"go").expect("release concurrent identity subprocesses");
    let bounded_outputs: Vec<_> = children
        .into_iter()
        .map(|child| collect_bounded(child, IDENTITY_TIMEOUT))
        .collect();
    assert!(
        all_ready,
        "not every identity subprocess reached the start gate"
    );

    let mut identities = Vec::with_capacity(CONCURRENT_FIRST_RUNS);
    for bounded in bounded_outputs {
        assert!(
            !bounded.timed_out,
            "concurrent identity subprocess timed out"
        );
        let identity = parse_identity(&bounded.output);
        assert!(
            bounded.output.stderr.is_empty(),
            "concurrent identity subprocess wrote to stderr"
        );
        identities.push(identity);
    }

    let first = identities
        .first()
        .expect("at least one concurrent identity result");
    assert_direct_code_shape(first);
    for identity in &identities[1..] {
        assert_same_identity(first, identity);
    }
    assert_exact_identity_secret_records(&sandbox);
}

fn run_identity(sandbox: &Sandbox, repair: bool) -> Output {
    let mut command = sandbox.command();
    command.args(["share", "identity", "--json"]);
    if repair {
        command.arg("--repair");
    }
    command
        .output()
        .expect("launch standalone se identity command")
}

fn parse_identity(output: &Output) -> PublicIdentity {
    assert!(
        output.status.success(),
        "identity command failed with status {:?}",
        output.status.code()
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("identity stdout must be JSON");
    PublicIdentity {
        device_id: json_string(&value, "device_id").to_string(),
        device_name: json_string(&value, "device_name").to_string(),
        fingerprint: json_string(&value, "fingerprint").to_string(),
        node_id: json_string(&value, "node_id").to_string(),
        direct_code: json_string(&value, "direct_code").to_string(),
    }
}

fn json_string<'a>(value: &'a serde_json::Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("identity JSON field {field} must be a string"))
}

fn assert_same_identity(left: &PublicIdentity, right: &PublicIdentity) {
    assert!(left.device_id == right.device_id, "device id changed");
    assert!(left.device_name == right.device_name, "device name changed");
    assert!(left.fingerprint == right.fingerprint, "fingerprint changed");
    assert!(left.node_id == right.node_id, "Iroh node id changed");
    assert!(left.direct_code == right.direct_code, "direct code changed");
}

fn assert_direct_code_shape(identity: &PublicIdentity) {
    let parts: Vec<_> = identity.direct_code.split('-').collect();
    assert_eq!(parts.len(), 6, "direct code has an unexpected shape");
    assert!(parts[0] == "SE" && parts[1] == "D3");
    assert_eq!(parts[2].len(), 24, "direct lookup id length changed");
    assert!(
        is_lower_hex(parts[2]),
        "direct lookup id is not lowercase hex"
    );
    assert_eq!(parts[3].len(), 64, "direct secret length changed");
    assert!(is_lower_hex(parts[3]), "direct secret is not lowercase hex");
    assert!(parts[4] == identity.fingerprint);
    assert!(parts[5] == identity.node_id);
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn fingerprint(node_id: &str) -> String {
    let digest = Sha256::digest(node_id.as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn secret_record_path(sandbox: &Sandbox, account: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(b"smart_explorer\0");
    hasher.update(account.as_bytes());
    let name: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    sandbox
        .app_data_path("secrets-v1")
        .join(format!("{name}.secret"))
}

fn share_worker_running(sandbox: &Sandbox) -> bool {
    let status = run_bounded(
        sandbox.command().args(["share", "status", "--json"]),
        Duration::from_secs(10),
    );
    assert!(!status.timed_out, "Share status timed out");
    assert_success(&status.output);
    serde_json::from_slice::<serde_json::Value>(&status.output.stdout)
        .expect("Share status stdout must be JSON")["running"]
        .as_bool()
        .expect("Share status running must be a boolean")
}

fn assert_exact_identity_secret_records(sandbox: &Sandbox) {
    let records = identity_secret_records(sandbox);
    assert_eq!(records.len(), 2, "expected exactly two identity secrets");
    for record in records {
        let metadata = fs::symlink_metadata(record).expect("inspect identity secret record");
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
    }
}

fn assert_no_identity_secret_records(sandbox: &Sandbox) {
    let directory = sandbox.app_data_path("secrets-v1");
    if !directory.exists() {
        return;
    }
    let metadata = fs::symlink_metadata(&directory).expect("inspect empty credential directory");
    assert!(metadata.file_type().is_dir());
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o700);
    assert!(
        identity_secret_records(sandbox).is_empty(),
        "failed identity lookup persisted a secret record"
    );
}

fn identity_secret_records(sandbox: &Sandbox) -> Vec<PathBuf> {
    let directory = sandbox.app_data_path("secrets-v1");
    let metadata = fs::symlink_metadata(&directory).expect("inspect Linux credential directory");
    assert!(metadata.file_type().is_dir());
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o700);
    fs::read_dir(directory)
        .expect("read Linux credential directory")
        .map(|entry| entry.expect("read Linux credential entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "secret")
        })
        .collect()
}

fn gated_identity_command(sandbox: &Sandbox, ready: &Path, gate: &Path) -> Command {
    let binary = std::env::var_os("SMART_EXPLORER_SE_BINARY")
        .unwrap_or_else(|| OsString::from(env!("CARGO_BIN_EXE_se")));
    let home = sandbox.path("home");
    let data = sandbox.path("app-data");
    let mut command = Command::new("/bin/sh");
    command
        .current_dir(&sandbox.root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", data.join("config"))
        .env("APPDATA", &data)
        .env("LOCALAPPDATA", &data)
        .args([
            "-c",
            r#": > "$1"
while [ ! -e "$2" ]; do sleep 0.01; done
exec "$3" share identity --json"#,
            "se-share-identity-gate",
        ])
        .arg(ready)
        .arg(gate)
        .arg(binary);
    for name in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_RUNTIME_DIR",
        "GNOME_KEYRING_CONTROL",
        "SSH_AUTH_SOCK",
    ] {
        command.env_remove(name);
    }
    command
}

fn wait_for_readiness(paths: &[PathBuf], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if paths.iter().all(|path| path.is_file()) {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

/// The `--sync-daemon` processes of one sandbox: guardians, and the workers
/// a guardian started (`SE_SYNC_GUARDIAN_CHILD`) with their parent process.
#[derive(Debug, Default)]
struct SandboxDaemons {
    guardians: Vec<u32>,
    workers: Vec<(u32, u32)>,
}

impl SandboxDaemons {
    fn is_one_supervised_worker(&self) -> bool {
        matches!(
            (self.guardians.as_slice(), self.workers.as_slice()),
            ([guardian], [(_, parent)]) if guardian == parent
        )
    }
}

fn sandbox_daemons(sandbox: &Sandbox) -> SandboxDaemons {
    let expected_data = format!("XDG_DATA_HOME={}", sandbox.path("app-data").display());
    let mut daemons = SandboxDaemons::default();
    let Ok(processes) = fs::read_dir("/proc") else {
        return daemons;
    };
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(command) = fs::read(process.path().join("cmdline")) else {
            continue;
        };
        if !command
            .split(|byte| *byte == 0)
            .any(|argument| argument == b"--sync-daemon")
        {
            continue;
        }
        let Ok(environment) = fs::read(process.path().join("environ")) else {
            continue;
        };
        let entries: Vec<&[u8]> = environment.split(|byte| *byte == 0).collect();
        if !entries.contains(&expected_data.as_bytes()) {
            continue;
        }
        if entries
            .iter()
            .any(|entry| entry.starts_with(b"SE_SYNC_GUARDIAN_CHILD="))
        {
            // A worker that ended between the reads has no parent left to show.
            if let Some(parent) = parent_pid(&process.path()) {
                daemons.workers.push((pid, parent));
            }
        } else {
            daemons.guardians.push(pid);
        }
    }
    daemons.guardians.sort_unstable();
    daemons.workers.sort_unstable();
    daemons
}

/// The fourth field of `/proc/<pid>/stat`, after the parenthesized name.
fn parent_pid(process: &Path) -> Option<u32> {
    let stat = fs::read_to_string(process.join("stat")).ok()?;
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(1)?.parse().ok()
}

struct DaemonStopGuard<'a> {
    sandbox: &'a Sandbox,
}

impl Drop for DaemonStopGuard<'_> {
    fn drop(&mut self) {
        let stop = self.sandbox.app_data_path("sync/daemon.stop");
        if let Some(parent) = stop.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(stop, b"stop");
        let heartbeat = self.sandbox.app_data_path("sync/daemon.heartbeat");
        let deadline = Instant::now() + Duration::from_secs(8);
        while heartbeat.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        self.wait_for_handoff_contenders();
    }
}

impl DaemonStopGuard<'_> {
    fn wait_for_handoff_contenders(&self) {
        let lock_path = self.sandbox.path("runtime/smart-explorer-sync-daemon.lock");
        if !lock_path.is_file() {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut quiet_since = None;
        while Instant::now() < deadline {
            let acquired = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
                .ok()
                .is_some_and(|file| {
                    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) == 0
                });
            if acquired {
                let quiet = quiet_since.get_or_insert_with(Instant::now);
                if quiet.elapsed() >= Duration::from_millis(500) {
                    return;
                }
            } else {
                quiet_since = None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
