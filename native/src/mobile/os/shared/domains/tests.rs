//! Domain facade tests on the Linux host: pure projections and file-level
//! helpers only (no worker, no Share host, no network).
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};

use super::job_json::{apply_draft, field_for_message, job_json, schedule_text};
use super::share_status::{status_json, StatusInput, WorkerFacts};
use crate::mobile::ApiError;
use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::{SyncJob, Trigger};

fn ok(result: Result<Value, ApiError>) -> Value {
    result.unwrap_or_else(|error| panic!("{}: {}", error.kind, error.message))
}

fn errors_of(job: Value) -> BTreeMap<String, String> {
    let value = ok(super::sync_jobs::validate(&json!({ "job": job })));
    serde_json::from_value(value["errors"].clone()).expect("errors map")
}

#[test]
fn android_task_job_json_round_trip_keeps_every_desktop_field() {
    let mut job = SyncJob::new(
        "Fotos".into(),
        "/storage/emulated/0/DCIM".into(),
        "sftp://u@nas:22/fotos".into(),
    );
    job.direction = crate::bisync::Direction::AtoB;
    job.conflict = crate::bisync::ConflictMode::NewerWins;
    job.delete_policy = crate::bisync::DeletePolicy::NoDelete;
    job.trigger = Trigger::Calendar;
    job.cal_time_min = 7 * 60 + 30;
    job.cal_weekdays = 0b0000_0101;
    job.active_from_min = 22 * 60;
    job.active_to_min = 6 * 60;
    job.ignore = vec!["*.tmp".into(), "cache/**".into()];
    job.max_delete = 50;
    job.max_delete_pct = 20;
    job.move_files = true;
    job.filter_min_size_kb = 4;
    job.bwlimit_kbps = 512;
    job.retries = 3;
    job.run_before = "echo vorher".into();

    let value = job_json(&job, None, None);
    assert_eq!(value["calendar"]["kind"], "weekly");
    assert_eq!(value["calendar"]["weekday"], 5);
    assert_eq!(value["schedule"], "Mo,Mi 07:30");

    let mut editor = JobEditor::from_job(&job);
    let errors = apply_draft(&mut editor, value.as_object().expect("job object"));
    assert!(errors.is_empty(), "{errors:?}");
    let rebuilt = editor.build_sync_job(Some(&job)).expect("valid job");
    assert_eq!(job_json(&rebuilt, None, None), value);
    assert_eq!(rebuilt.id, job.id);
    assert_eq!(rebuilt.filter_min_size_kb, 4);
    assert_eq!(rebuilt.bwlimit_kbps, 512);
    assert_eq!(rebuilt.retries, 3);
}

#[test]
fn android_task_job_validation_reports_desktop_errors_per_field() {
    assert!(errors_of(json!({ "source": "/data", "target": "/backup" })).is_empty());
    let nested = errors_of(json!({ "source": "/data", "target": "/data/sub" }));
    assert!(nested.contains_key("target"), "{nested:?}");
    let interval = errors_of(json!({
        "source": "/data", "target": "/backup", "trigger": "interval", "intervalMin": 0,
    }));
    assert!(interval.contains_key("intervalMin"), "{interval:?}");
    let window =
        errors_of(json!({ "source": "/data", "target": "/backup", "activeFromMin": 5000 }));
    assert!(window.contains_key("activeFromMin"), "{window:?}");
    let weekly = errors_of(json!({
        "source": "/data", "target": "/backup", "trigger": "calendar",
        "calendar": { "kind": "weekly", "minuteOfDay": 60, "weekday": 0, "monthday": 0 },
    }));
    assert!(weekly.contains_key("calendar"), "{weekly:?}");
    let unknown =
        errors_of(json!({ "source": "/data", "target": "/backup", "direction": "sideways" }));
    assert!(unknown.contains_key("direction"), "{unknown:?}");
    let internal = errors_of(json!({ "source": "trash://", "target": "/backup" }));
    assert!(internal.contains_key("source"), "{internal:?}");
    let missing = errors_of(json!({ "source": "", "target": "/backup" }));
    assert!(missing.contains_key("source"), "{missing:?}");
}

#[test]
fn android_task_editor_messages_map_to_their_fields() {
    assert_eq!(
        field_for_message("Beginn der aktiven Zeit ist keine gültige Uhrzeit."),
        "activeFromMin"
    );
    assert_eq!(
        field_for_message("Uhrzeit muss als HH:MM angegeben werden."),
        "calendar"
    );
    assert_eq!(
        field_for_message("Der prozentuale Lösch-Schutz darf höchstens 100 sein."),
        "maxDeletePct"
    );
    assert_eq!(
        field_for_message("Lösch-Schutz enthält keine gültige nichtnegative Zahl."),
        "maxDelete"
    );
    assert_eq!(field_for_message("Ungültiges Setup: irgendwas"), "job");
}

#[test]
fn android_task_sync_options_list_every_mode_without_device_triggers() {
    let options = ok(super::sync_jobs::options());
    let values = |key: &str| -> Vec<String> {
        options[key]
            .as_array()
            .expect("list")
            .iter()
            .map(|entry| entry["value"].as_str().expect("value").to_string())
            .collect()
    };
    assert_eq!(values("conflicts").len(), 8);
    assert_eq!(values("versionings").len(), 4);
    assert_eq!(values("directions"), ["a2b", "b2a", "both"]);
    assert!(!values("triggers").contains(&"onconnect".to_string()));
    assert!(values("triggers").contains(&"onstartup".to_string()));
    assert_eq!(values("calendarKinds"), ["daily", "weekly", "monthly"]);
    assert_eq!(options["defaults"]["id"], "");
    assert_eq!(options["defaults"]["direction"], "both");
}

#[test]
fn android_task_schedule_text_matches_the_desktop_list() {
    let mut job = SyncJob::new("x".into(), "/a".into(), "/b".into());
    assert_eq!(schedule_text(&job), "manuell");
    job.trigger = Trigger::Interval;
    job.interval_min = 60;
    assert_eq!(schedule_text(&job), "alle 60 min");
    job.trigger = Trigger::Calendar;
    job.cal_time_min = 9 * 60;
    job.cal_monthday = 15;
    assert_eq!(schedule_text(&job), "monatl. am 15. um 09:00");
}

#[test]
fn android_task_update_feed_reads_version_checksum_and_apk() {
    let feed = tempfile::tempdir().expect("feed dir");
    let apk = b"not really an apk".repeat(100);
    let hash = {
        let path = feed.path().join(super::update::APK_NAME);
        std::fs::write(&path, &apk).expect("apk");
        crate::updater::file_sha256(&path).expect("hash")
    };
    std::fs::write(feed.path().join("version.txt"), "0.5.200\n").expect("version");
    std::fs::write(
        feed.path()
            .join(format!("{}.sha256", super::update::APK_NAME)),
        format!("{hash}  {}\n", super::update::APK_NAME),
    )
    .expect("sidecar");
    let feed_path = feed.path().to_string_lossy().into_owned();
    assert_eq!(
        crate::updater::read_feed_version(&feed_path).expect("version"),
        "0.5.200"
    );
    assert_eq!(
        crate::updater::read_feed_sha256(&feed_path, super::update::APK_NAME).expect("sha"),
        hash
    );
    assert!(crate::updater::is_newer("0.5.200", "0.5.163"));
    assert!(!crate::updater::is_newer("0.5.163", "0.5.163"));

    let target = tempfile::tempdir().expect("target dir");
    let dest = target.path().join("update.apk");
    let mut last: (u64, Option<u64>) = (0, None);
    crate::updater::download_feed_file(
        &feed_path,
        super::update::APK_NAME,
        &dest,
        &AtomicBool::new(false),
        &mut |done: u64, total: Option<u64>| last = (done, total),
    )
    .expect("download");
    assert_eq!(std::fs::read(&dest).expect("downloaded"), apk);
    assert_eq!(last, (apk.len() as u64, Some(apk.len() as u64)));
    assert_eq!(crate::updater::file_sha256(&dest).expect("hash"), hash);

    let canceled = target.path().join("canceled.apk");
    assert!(crate::updater::download_feed_file(
        &feed_path,
        super::update::APK_NAME,
        &canceled,
        &AtomicBool::new(true),
        &mut |_: u64, _: Option<u64>| {},
    )
    .is_err());
    assert!(!canceled.exists());
    assert!(!target.path().join("canceled.apk.part").exists());

    std::fs::write(feed.path().join("version.txt"), "<html>\n").expect("broken");
    assert!(crate::updater::read_feed_version(&feed_path).is_err());
    assert!(super::update::builtin_feed().starts_with("https://"));
}

#[test]
fn android_task_analysis_node_lists_children_by_size_with_locations() {
    let root = tempfile::tempdir().expect("root");
    std::fs::create_dir(root.path().join("sub")).expect("sub");
    std::fs::write(root.path().join("sub/small.txt"), vec![b'a'; 100]).expect("small");
    std::fs::write(root.path().join("big.bin"), vec![b'b'; 3000]).expect("big");
    let root_text = root.path().to_string_lossy().into_owned();
    let outcome = crate::analytics::scan(root.path(), &crate::analytics::Progress::default());
    super::analyze::insert_analysis_for_test("t-analysis", outcome, &root_text, &root_text);

    let top = ok(super::analyze::node(
        &json!({ "taskId": "t-analysis", "path": [] }),
    ));
    assert_eq!(top["size"], 3100);
    let names: Vec<&str> = top["children"]
        .as_array()
        .expect("children")
        .iter()
        .map(|child| child["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["big.bin", "sub"]);
    assert_eq!(top["children"][1]["childCount"], 1);
    let sub = ok(super::analyze::node(
        &json!({ "taskId": "t-analysis", "path": ["sub"] }),
    ));
    assert_eq!(sub["location"], format!("{root_text}/sub"));
    assert_eq!(sub["children"][0]["isDir"], false);
    assert!(super::analyze::node(&json!({ "taskId": "t-analysis", "path": ["gone"] })).is_err());
    assert!(super::analyze::node(&json!({ "taskId": "unknown", "path": [] })).is_err());
    let issues = ok(super::analyze::issues(&json!({ "taskId": "t-analysis" })));
    assert_eq!(issues["count"], 0);
}

fn snapshot_profiles() -> crate::share::ShareProfiles {
    let mut profiles = crate::share::ShareProfiles::default();
    profiles.direct_contacts.push(
        serde_json::from_value(json!({
            "id": "c1", "display_name": "Laptop", "lookup_id": "l1",
            "expected_fingerprint": "fp", "auto_connect": true, "auto_open": false,
            "status": "Available",
        }))
        .expect("contact"),
    );
    profiles.rooms.push(
        serde_json::from_value(json!({
            "id": "r1", "name": "Team", "room_id": "wire-room", "auto_join": true,
            "members": [{
                "device_id": "d1", "device_name": "Tablet", "fingerprint": "fp2",
                "public_key": "pk", "candidates": [],
            }],
        }))
        .expect("room"),
    );
    profiles
        .default_direct_exports
        .roots
        .push(crate::share::SharedRoot {
            label: "Intern".into(),
            path: "/storage/emulated/0".into(),
        });
    profiles
}

#[test]
fn android_task_share_status_maps_a_worker_snapshot() {
    let snapshot = crate::daemon::ShareWorkerSnapshot {
        profiles: snapshot_profiles(),
        running: true,
        connected: true,
        relay_url: "https://relay.example".into(),
        ..Default::default()
    };
    let worker = WorkerFacts::from_snapshot(&snapshot);
    let discovery = crate::share::discovery_state::DiscoveryUiState::default();
    let notices = VecDeque::from(vec!["Share-Server verbunden".to_string()]);
    let status = status_json(&StatusInput {
        worker: Some(&worker),
        profiles: &snapshot.profiles,
        identity: None,
        server: " wss://share.example ",
        poll_error: None,
        discovery: &discovery,
        offer_aliases: &BTreeMap::new(),
        last_exchange: None,
        notices: &notices,
        now_secs: 1_700_000_000,
    });
    assert_eq!(status["running"], true);
    assert_eq!(status["connected"], true);
    assert_eq!(status["relayUrl"], "https://relay.example");
    assert_eq!(status["server"], "wss://share.example");
    assert_eq!(status["lastError"], Value::Null);
    assert_eq!(status["identity"]["deviceId"], "");
    let device = &status["devices"][0];
    assert_eq!(device["location"], "share://direct/c1");
    assert_eq!(device["status"], "available");
    assert_eq!(device["online"], true);
    assert_eq!(device["lan"], false);
    let room = &status["rooms"][0];
    assert_eq!(room["profileId"], "r1");
    assert_eq!(room["members"][0]["location"], "share://room/r1/d1");
    assert_eq!(
        status["exports"]["direct"][0]["path"],
        "/storage/emulated/0"
    );
    assert_eq!(status["exports"]["rooms"]["r1"], json!([]));
    assert_eq!(status["incoming"], json!([]));
    assert_eq!(status["discovery"]["offer"], Value::Null);
    assert_eq!(status["notices"][0], "Share-Server verbunden");

    let offline = status_json(&StatusInput {
        worker: None,
        profiles: &snapshot.profiles,
        identity: None,
        server: "",
        poll_error: Some("Background-Worker ist nicht bereit"),
        discovery: &discovery,
        offer_aliases: &BTreeMap::new(),
        last_exchange: None,
        notices: &VecDeque::new(),
        now_secs: 1_700_000_000,
    });
    assert_eq!(offline["running"], false);
    assert_eq!(offline["server"], Value::Null);
    assert_eq!(offline["lastError"], "Background-Worker ist nicht bereit");
    assert_eq!(offline["lanPresence"], "aus");
}

#[test]
fn android_task_share_server_rules_match_the_desktop() {
    use super::share_settings::validate_server;
    assert_eq!(
        validate_server(" wss://a.example:443, tcp://b:7000 ").expect("valid"),
        "wss://a.example:443, tcp://b:7000"
    );
    assert!(validate_server("user@host:1").is_err());
    assert!(validate_server("ftp://host").is_err());
    assert!(validate_server("a b").is_err());
    assert!(validate_server(" ; ").is_err());
}

#[test]
fn android_task_exec_command_split_respects_quotes() {
    use super::share_requests::split_command;
    assert_eq!(
        split_command(r#"ls -la "my dir" 'a b' c\ d """#).expect("split"),
        ["ls", "-la", "my dir", "a b", "c d", ""]
    );
    assert!(split_command("echo 'open").is_err());
    assert!(split_command("echo \\").is_err());
}

#[test]
fn android_task_forget_host_key_removes_only_that_entry() {
    let dir = tempfile::tempdir().expect("dir");
    let store = dir.path().join("known_hosts_sftp.txt");
    std::fs::write(&store, "nas:22 SHA256:aaa\nnas:2222 SHA256:bbb\n").expect("store");
    assert!(super::host_keys::forget_in(dir.path(), "nas:22").expect("forget"));
    assert_eq!(
        std::fs::read_to_string(&store).expect("read"),
        "nas:2222 SHA256:bbb\n"
    );
    assert!(!super::host_keys::forget_in(dir.path(), "nas:22").expect("again"));
    let empty = tempfile::tempdir().expect("empty");
    assert!(!super::host_keys::forget_in(empty.path(), "nas:22").expect("missing store"));
}

#[test]
fn android_task_locations_keep_the_endpoint_prefix() {
    use super::locations::location_for;
    assert_eq!(
        location_for("/storage/emulated/0", "/storage/emulated/0/a"),
        "/storage/emulated/0/a"
    );
    assert_eq!(
        location_for("sftp://u@nas:22/data", "/data/x"),
        "sftp://u@nas:22/data/x"
    );
    assert_eq!(
        location_for("webdav://u@[::1]:443/", "/d"),
        "webdav://u@[::1]:443/d"
    );
    assert_eq!(
        location_for("gdrive:///Fotos", "/Fotos/a.jpg"),
        "gdrive:///Fotos/a.jpg"
    );
    assert_eq!(
        location_for("share://direct/c1", "/Bilder"),
        "share://direct/c1/Bilder"
    );
    assert_eq!(
        location_for("share://room/r1/d1/x", "/x/y"),
        "share://room/r1/d1/x/y"
    );
}

#[test]
fn android_task_worker_log_tail_starts_at_a_line() {
    use super::background::tail_bytes;
    assert_eq!(tail_bytes("eins\nzwei\ndrei", 100), "eins\nzwei\ndrei");
    assert_eq!(tail_bytes("eins\nzwei\ndrei", 7), "drei");
    assert_eq!(tail_bytes("äöü", 3), "ü");
}
