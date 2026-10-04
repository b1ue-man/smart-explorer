//! C04: real persisted provider locators, complete byte oracles, no fallback passes.
use super::sync_reliability_task_provider_fixture::{self as fixture, Provider};
use crate::bisync::{self, BisyncOptions, CompareMode, Direction, RunRequest, RunSettings, ScanDepth};
use crate::vfs::{Backend, BackendHandle, CachingBackend};
use std::sync::{atomic::AtomicBool, Arc};

fn run(a: &dyn Backend, ra: &str, b: &dyn Backend, rb: &str, owner: &str,
       direction: Direction) -> bisync::Outcome {
    let ignore = bisync::empty_globset();
    let filter = bisync::WalkFilter::basic(true, &ignore);
    let cancel = AtomicBool::new(false);
    let opts = BisyncOptions { compare: CompareMode::Checksum, direction, ..Default::default() };
    let mut request = RunRequest::new(a, ra, b, rb, opts, &filter, &cancel);
    request.settings = RunSettings { depth: ScanDepth::Full, ..RunSettings::for_job(owner) };
    let out = bisync::run_with(request);
    assert!(out.errors.is_empty(), "{owner}: {:?}", out.errors);
    assert_eq!(out.stats.errors, 0, "{owner}");
    assert!(out.conflicts.is_empty(), "{owner}: unresolved conflict");
    assert!(!out.busy && !out.canceled, "{owner}: incomplete run");
    assert!(out.blocked.is_none() && out.stopped.is_none(), "{owner}: stopped");
    assert!(out.deferred.is_empty(), "{owner}: deferred writes");
    assert!(out.omissions.is_empty(), "{owner}: protected omissions {:?}",
        out.omissions.reported().collect::<Vec<_>>());
    out
}

fn assert_bytes(a: &dyn Backend, ra: &str, b: &dyn Backend, rb: &str,
                rel: &str, expected: &[u8]) {
    assert_eq!(fixture::read(a, ra, rel), expected, "source bytes: {rel}");
    assert_eq!(fixture::read(b, rb, rel), expected, "target bytes: {rel}");
}

fn pair(left: &Provider, right: &Provider, index: usize) {
    let child = format!("pair-{index:03}");
    let (raw_a, ra) = left.open(&child);
    let right_child = if left.name == right.name { format!("{child}-other") } else { child };
    let (raw_b, rb) = right.open(&right_child);
    // Peers resolve through their real IPC agent and guard. Caching wrappers
    // must retain precisely that provider identity and observe external writes.
    let a: BackendHandle = Arc::new(CachingBackend::new(raw_a.clone()));
    let b: BackendHandle = Arc::new(CachingBackend::new(raw_b.clone()));
    assert_eq!(a.state_identity(), raw_a.state_identity());
    assert_eq!(b.state_identity(), raw_b.state_identity());
    if left.name != right.name && left.name != "unc" && left.name != "mapped"
        && right.name != "unc" && right.name != "mapped" {
        assert_ne!(a.state_identity(), b.state_identity(), "distinct providers collapsed");
    }
    let owner = format!("sync-provider-{index}-{}-{}", left.name, right.name);
    fixture::write(&*raw_a, &ra, ".obsidian/preferences.json", b"left initial notebook\n");
    fixture::write(&*raw_b, &rb, "literal%20-file.txt", b"right initial literal\n");
    a.list_dir(&ra).expect("prime source browsing cache");
    b.list_dir(&rb).expect("prime target browsing cache");
    run(&*a, &ra, &*b, &rb, &owner, Direction::Both);
    assert_bytes(&*raw_a, &ra, &*raw_b, &rb, ".obsidian/preferences.json", b"left initial notebook\n");
    assert_bytes(&*raw_a, &ra, &*raw_b, &rb, "literal%20-file.txt", b"right initial literal\n");
    fixture::write(&*raw_a, &ra, ".obsidian/preferences.json", b"left changed notebook with new bytes\n");
    fixture::write(&*raw_b, &rb, "literal%20-file.txt", b"right changed literal with new bytes\n");
    run(&*a, &ra, &*b, &rb, &owner, Direction::Both);
    assert_bytes(&*raw_a, &ra, &*raw_b, &rb, ".obsidian/preferences.json", b"left changed notebook with new bytes\n");
    assert_bytes(&*raw_a, &ra, &*raw_b, &rb, "literal%20-file.txt", b"right changed literal with new bytes\n");
    fixture::write(&*raw_b, &rb, ".obsidian/preferences.json", b"counterpart edits former left file\n");
    fixture::write(&*raw_a, &ra, "literal%20-file.txt", b"counterpart edits former right file\n");
    run(&*a, &ra, &*b, &rb, &owner, Direction::Both);
    assert_bytes(&*raw_a, &ra, &*raw_b, &rb, ".obsidian/preferences.json", b"counterpart edits former left file\n");
    assert_bytes(&*raw_a, &ra, &*raw_b, &rb, "literal%20-file.txt", b"counterpart edits former right file\n");
    let quiet = run(&*a, &ra, &*b, &rb, &owner, Direction::Both);
    assert_eq!((quiet.stats.a_to_b, quiet.stats.b_to_a, quiet.stats.deleted, quiet.stats.bytes),
               (0, 0, 0, 0), "{owner}: no-op transferred or deleted");
    assert!(quiet.baseline.contains_key(".obsidian/preferences.json"));
    assert!(quiet.baseline.contains_key("literal%20-file.txt"));
    println!("C04 pair={}+{} seed/change/counterchange/noop bytes=confirmed identities={:?}|{:?}",
             left.name, right.name, raw_a.state_identity(), raw_b.state_identity());
}

#[test]
fn sync_reliability_task_provider_matrix() {
    let mut providers = fixture::providers();
    let required: &[&str] = if cfg!(windows) { &["local", "unc", "mapped", "direct", "room"] }
        else { &["local", "sftp", "sftp-other", "sftp-key", "agent", "ftp", "ftps", "webdav", "smb", "direct", "room"] };
    for name in required {
        assert!(providers.iter().any(|provider| provider.name == *name), "C04 missing {name}");
    }
    // UNC sessions are process-owned leases. Keep the real authenticated
    // connection alive while its mapped alias participates in later pairs.
    let _unc_lease = providers.iter().find(|provider| provider.name == "unc")
        .map(|provider| provider.open("connection-lease"));
    let drive = crate::gdrive::sync_reliability_task_fixture::DriveFixture::new("/Notebook");
    let (drive_backend, drive_root) = drive.endpoint();
    let locator = "gdrive:///Notebook".to_string();
    let _drive_resolver = crate::daemon::sync_reliability_task_old_jobs_tests::EndpointFixtures::new(
        vec![(locator.clone(), drive_backend, drive_root)]);
    providers.push(Provider { name: "drive".into(), endpoint: locator, agent: false });
    let mut index = 0;
    for (left_index, left) in providers.iter().enumerate() {
        for right in providers.iter().skip(left_index) {
            // UNC and its mapped alias intentionally represent the same share.
            if matches!((left.name.as_str(), right.name.as_str()), ("unc", "mapped")) { continue; }
            pair(left, right, index);
            index += 1;
        }
    }
    // Same relative root on two SFTP authorities must remain independently writable.
    if !cfg!(windows) {
        let left = providers.iter().find(|p| p.name == "sftp").unwrap();
        let right = providers.iter().find(|p| p.name == "sftp-other").unwrap();
        let (a, ra) = left.open("authority-isolation");
        let (b, rb) = right.open("authority-isolation");
        assert_eq!(ra, rb);
        assert_ne!(a.namespace_identity(), b.namespace_identity());
        fixture::write(&*a, &ra, "same.txt", b"first authority bytes");
        fixture::write(&*b, &rb, "same.txt", b"second authority bytes");
        assert_eq!(fixture::read(&*a, &ra, "same.txt"), b"first authority bytes");
        assert_eq!(fixture::read(&*b, &rb, "same.txt"), b"second authority bytes");
    }
    let zipped = fixture::zip();
    let local = providers.iter().find(|p| p.name == "local").unwrap();
    let (target, root) = local.open("zip-source");
    let bytes = b"{\"zip\":\"readonly source\"}\n";
    assert_eq!(fixture::read(&zipped, "", ".obsidian/preferences.json"), bytes);
    assert!(zipped.open_write("denied.txt").is_err(), "ZIP must remain read-only");
    run(&zipped, "", &*target, &root, "sync-provider-zip-source", Direction::AtoB);
    assert_eq!(fixture::read(&*target, &root, ".obsidian/preferences.json"), bytes);
    let quiet = run(&zipped, "", &*target, &root, "sync-provider-zip-source", Direction::AtoB);
    assert_eq!((quiet.stats.a_to_b, quiet.stats.b_to_a, quiet.stats.bytes), (0, 0, 0));
    println!("C04 readonly ZIP source bytes=confirmed; provider pairs={index}");
}
