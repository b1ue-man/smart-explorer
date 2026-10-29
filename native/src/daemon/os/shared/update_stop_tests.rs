//! DSK-M2: one update-stop rule for `se mcp`, every GUI window and every
//! background process, while the updater helper itself stays unchanged.
use super::{is_new_request, UpdateStopBaseline, UpdateStopSource};
use sha2::{Digest, Sha256};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type FakeMarker = Arc<Mutex<Option<SystemTime>>>;

fn fake_marker(initial: Option<SystemTime>) -> (FakeMarker, UpdateStopSource) {
    let marker = Arc::new(Mutex::new(initial));
    let reader = marker.clone();
    let source: UpdateStopSource = Arc::new(move || *reader.lock().unwrap());
    (marker, source)
}

fn set(marker: &FakeMarker, value: Option<SystemTime>) {
    *marker.lock().unwrap() = value;
}

#[test]
fn desktop_task_update_stop_rule_matches_mcp() {
    let old = Some(UNIX_EPOCH + Duration::from_secs(10));
    let new = Some(UNIX_EPOCH + Duration::from_secs(20));
    // Every pair of marker states at start and later. The rows of
    // `mcp_task_only_new_update_stop_requests_end_the_server` are a subset.
    let table = [
        (None, None, false),
        (None, old, true),
        (None, new, true),
        (old, None, false),
        (old, old, false),
        (old, new, true),
        (new, None, false),
        (new, old, true),
        (new, new, false),
    ];
    for (baseline, current, expected) in table {
        assert_eq!(
            is_new_request(baseline, current),
            expected,
            "{baseline:?} -> {current:?}"
        );
        // The se shell (`cli/shell_repl.rs`, `cli/shell_terminal.rs`) states
        // the same rule as `current.is_some() && current != baseline`.
        assert_eq!(current.is_some() && current != baseline, expected);
        // The baseline object applies the same rule to its marker source.
        let (marker, source) = fake_marker(baseline);
        let armed = UpdateStopBaseline::with_source(source);
        set(&marker, current);
        assert_eq!(armed.requested(), expected, "{baseline:?} -> {current:?}");
    }
}

#[test]
fn update_stop_watch_fires_once_and_ends_with_its_handle() {
    let stale = Some(UNIX_EPOCH + Duration::from_secs(5));
    let (marker, source) = fake_marker(stale);
    let (sender, received) = mpsc::channel();
    let watch = UpdateStopBaseline::with_source(source)
        .spawn_watch(Duration::from_millis(10), move || {
            let _ = sender.send(());
        })
        .unwrap();
    // The marker present at start is not a request.
    assert_eq!(
        received.recv_timeout(Duration::from_millis(150)),
        Err(RecvTimeoutError::Timeout)
    );
    assert!(!watch.fired());
    set(&marker, Some(UNIX_EPOCH + Duration::from_secs(6)));
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(watch.fired());
    // The callback ran once and the thread ended with it.
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)),
        Err(RecvTimeoutError::Disconnected)
    );
    watch.stop();

    // Dropping the handle ends the thread long before its first read.
    let (_marker, source) = fake_marker(None);
    let (sender, received) = mpsc::channel::<()>();
    let watch = UpdateStopBaseline::with_source(source)
        .spawn_watch(Duration::from_secs(3600), move || {
            let _ = sender.send(());
        })
        .unwrap();
    drop(watch);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)),
        Err(RecvTimeoutError::Disconnected)
    );
}

/// Erhaltungsfall F1: the updater helper is byte-identical to the released
/// `v0.5.188` (LF line endings); only the processes learn to honour the
/// marker. The pin moved from `da2646d2` with the intended updater changes
/// up to the release (16052ba5 with installer.rs and mcp.rs, 10988167,
/// 7597ce12); this batch leaves the updater untouched.
#[test]
fn desktop_task_updater_helper_unchanged() {
    const EXPECTED: [(&str, &str); 20] = [
        (
            "smart_explorer_updater.rs",
            "6f6621dd99f4b3ed9493b1137083d4054c22723a0f39a1f54be53efefe44a712",
        ),
        (
            "smart_explorer_updater/archive.rs",
            "3b861d34b9c421cba5ecfde59b565af704cca4d9018a2bff8e25ec88bc3675b4",
        ),
        (
            "smart_explorer_updater/args.rs",
            "4ffbf65d145946fe5db1d9bd22a571f0514e5b75d12c8e54e5a2127b6a881e96",
        ),
        (
            "smart_explorer_updater/bookkeeping.rs",
            "02e1acf514a40796adb29d619db4dca27cf856525770ffc476ce4131246364a9",
        ),
        (
            "smart_explorer_updater/hash.rs",
            "a7bdc6013383eafe115a9bd5ba5d4b396010ca3a781eaca7dd1b511ee4d98d84",
        ),
        (
            "smart_explorer_updater/installer.rs",
            "30b16569ea4cfa8a5bb1993fc96640b2c57d083cbfe65e2b3935f02ab2e2bbe3",
        ),
        (
            "smart_explorer_updater/instance.rs",
            "6d6f6bfcc653d8b94207ed67df21b023cd2bf35c8a04efa844219d03e3bff08c",
        ),
        (
            "smart_explorer_updater/launch.rs",
            "f466d8e1811e5ca7447b6b6da8c7b6aaf606c0a5d2c6c0464105f6e35564199c",
        ),
        (
            "smart_explorer_updater/legacy.rs",
            "5f0c35cfebda43bfdc9928b426bf93d1355e963a5b6f5b2cd1acbe54fad62efe",
        ),
        (
            "smart_explorer_updater/legacy_intent.rs",
            "d3d4f4d60bb006c78d25e0cc9776638bffde4eac2eabe541cfe1a79c2df90177",
        ),
        (
            "smart_explorer_updater/legacy_recovery.rs",
            "1255bda564afdd1c1a30dd738e3d7fd5775359261121f7884396faedee1c4be6",
        ),
        (
            "smart_explorer_updater/legacy_tests.rs",
            "629cde47fccdd719805508945f90cc263b8a28ec5f89d98e30c7605c9de80be8",
        ),
        (
            "smart_explorer_updater/logging.rs",
            "2d5143edde328e97c8b1201ff94f61062e01133ba4f9037f9331c280d9eb35ee",
        ),
        (
            "smart_explorer_updater/mcp.rs",
            "54b5739ec8078bda225453abf47a0127c7e79aa1e2d4131e1a9651423e1521b5",
        ),
        (
            "smart_explorer_updater/parent.rs",
            "00997597960ebbe0b7a8c20f0f3dcd6a2fd3524a6dc02fb83a3023ed767e3579",
        ),
        (
            "smart_explorer_updater/path_safety.rs",
            "13f4a5c070abcb5775e6e0d923b344f430b5007f736aecd282007a05e3d3378b",
        ),
        (
            "smart_explorer_updater/process.rs",
            "f24ff0e4245f4bfdc5b392e43f75ba1a20aa7a62c67ae398edfc76da38efa83c",
        ),
        (
            "smart_explorer_updater/replace.rs",
            "b5366a6516cd1c746d6f049ae9de4602ed7fc182ff8f92e6731ee85d601505e9",
        ),
        (
            "smart_explorer_updater/replace_platform.rs",
            "8bdeb5cb36124d992a1d785a1ad6126896a3feca430248e03ca2c2d3abb857c3",
        ),
        (
            "smart_explorer_updater/replace_tests.rs",
            "539f86c9a42bd4246b862c958131bd854513ca420a673e4c30e96a168cd68b9b",
        ),
    ];
    let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("bin");
    let mut found: Vec<String> = std::fs::read_dir(bin.join("smart_explorer_updater"))
        .unwrap()
        .map(|entry| {
            let name = entry.unwrap().file_name();
            format!("smart_explorer_updater/{}", name.to_string_lossy())
        })
        .collect();
    found.push("smart_explorer_updater.rs".into());
    found.sort();
    let mut expected: Vec<String> = EXPECTED.iter().map(|(name, _)| name.to_string()).collect();
    expected.sort();
    assert_eq!(found, expected, "file set of the updater helper changed");
    for (name, sha256) in EXPECTED {
        let text = std::fs::read_to_string(bin.join(name)).unwrap();
        // A Windows checkout may convert the line endings of the LF sources.
        let normalized = text.replace("\r\n", "\n");
        assert_eq!(
            format!("{:x}", Sha256::digest(normalized.as_bytes())),
            sha256,
            "{name} changed"
        );
    }
}
