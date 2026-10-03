use super::{access_task::DeniedDirectory, broker, image_lock::LockedImage, pipe::Pipe};
use crate::local_access::protocol::{ReadKind, ReadReply, ReadRequest, Startup, PIPE_PREFIX};
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::io::{AsHandle, AsRawHandle},
    process::{Child, Command},
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "child endpoint, launched only by the task's authenticated IPC fixture"]
fn read_helper_process_fixture() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_READ_HELPER_CHILD").as_deref(),
        Ok("1")
    );
    broker::serve(Startup {
        pipe: std::env::var("SMART_EXPLORER_READ_HELPER_PIPE").unwrap(),
        parent: std::env::var("SMART_EXPLORER_READ_HELPER_PARENT")
            .unwrap()
            .parse()
            .unwrap(),
        image_sha256: std::env::var("SMART_EXPLORER_READ_HELPER_HASH").unwrap(),
        root: std::env::var("SMART_EXPLORER_READ_HELPER_ROOT").unwrap(),
    })
    .unwrap();
}

#[test]
#[ignore = "real Windows process, backup privilege and ACL fixture: remote runner only"]
fn search_recursive_access_task_authenticated_helper_reuses_read_handles_in_parent() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_ANALYTICS_TASK").as_deref(),
        Ok("1")
    );
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("consented");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("asset.blend");
    std::fs::write(&source, b"protected payload").unwrap();
    let child_dir = root.join("protected-child");
    std::fs::create_dir(&child_dir).unwrap();
    std::fs::write(child_dir.join("inside.txt"), b"protected child payload").unwrap();
    std::fs::write(fixture.path().join("outside.txt"), b"outside").unwrap();
    let denied = DeniedDirectory::new(&root);
    let denied_file = DeniedDirectory::new(&source);
    let denied_child = DeniedDirectory::new(&child_dir);
    denied.assert_ordinary_denied();
    denied_file.assert_file_read_denied();
    let root_text = root.to_string_lossy().replace('\\', "/");
    let image = LockedImage::current().unwrap();
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).unwrap();
    let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let pipe_name = format!("{PIPE_PREFIX}{suffix}");
    let pipe = Pipe::server(&pipe_name).unwrap();
    let mut process = Process(
        Command::new(std::env::current_exe().unwrap())
            .stdout(std::process::Stdio::null())
            .args([
                "--exact",
                "local_access::platform::helper_task::read_helper_process_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("SMART_EXPLORER_READ_HELPER_CHILD", "1")
            .env("SMART_EXPLORER_READ_HELPER_PIPE", &pipe_name)
            .env(
                "SMART_EXPLORER_READ_HELPER_PARENT",
                std::process::id().to_string(),
            )
            .env("SMART_EXPLORER_READ_HELPER_HASH", &image.hash)
            .env("SMART_EXPLORER_READ_HELPER_ROOT", &root_text)
            .spawn()
            .unwrap(),
    );
    let peer = process.0.as_raw_handle();
    pipe.accept(peer).unwrap();
    assert_eq!(pipe.peer_id(true).unwrap(), process.0.id());
    let ready: ReadReply = pipe.receive(peer, Duration::from_secs(30)).unwrap();
    assert_eq!(ready.error, None);
    assert_eq!(ready.handle, 0);
    for path in [
        fixture
            .path()
            .join("outside.txt")
            .to_string_lossy()
            .into_owned(),
        format!("{root_text}/../outside.txt"),
    ] {
        pipe.send(
            &ReadRequest {
                path,
                kind: ReadKind::File,
            },
            peer,
        )
        .unwrap();
        let refused: ReadReply = pipe.receive(peer, Duration::from_secs(30)).unwrap();
        assert_eq!(refused.handle, 0);
        assert!(refused.error.is_some());
    }
    let child = process.0.as_handle().try_clone_to_owned().unwrap();
    broker::install(root_text.clone(), pipe, child).unwrap();
    assert!(broker::granted(&root_text));
    // A foreign/ordinary scan cannot borrow a GUI grant. The explicitly
    // consented scan keeps the same read-only authority through child pins.
    {
        let _identity = super::access_task::Identity::new(true);
        assert!(super::DirectoryHandle::open_root(&root).is_err());
        assert!(super::DirectoryHandle::open_root_consented(&root).is_err());
    }
    // Exercise the authenticated PinRoot and PinChild protocol even when the
    // runner itself also owns backup privilege and could open them directly.
    let root_pins = broker::pin_granted(&root, true).unwrap().unwrap();
    assert!(!root_pins.files.is_empty());
    let child_pins = broker::pin_granted(&child_dir, false).unwrap().unwrap();
    assert_eq!(child_pins.files.len(), 1);
    // Use the very constructor used by open_root_consented's broker fallback.
    // This ensures the fixture exercises the RPC branch on elevated runners.
    let pinned = super::DirectoryHandle::from_root_pins(
        root_pins, super::normalize_scan_root(&root),
    ).unwrap();
    drop(super::DirectoryHandle::open_root_consented(&root).unwrap());
    let names: Vec<_> = pinned.read_directory().unwrap()
        .map(|entry| entry.unwrap().name).collect();
    assert!(names.iter().any(|name| name == "asset.blend"));
    let child_pin = pinned.open_child(std::ffi::OsStr::new("protected-child")).unwrap();
    let mut child_file = child_pin.open_regular_child(std::ffi::OsStr::new("inside.txt")).unwrap();
    let mut payload = String::new();
    child_file.read_to_string(&mut payload).unwrap();
    assert_eq!(payload, "protected child payload");
    assert!(child_file.write_all(b"cannot write").is_err());
    assert!(std::fs::rename(&child_dir, root.join("moved-child")).is_err());
    for _ in 0..2 {
        let mut file: File = broker::open_granted(&source, ReadKind::File)
            .unwrap()
            .unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();
        assert_eq!(content, "protected payload");
        assert!(file.write_all(b"cannot write").is_err());
    }
    assert!(broker::open_granted(&fixture.path().join("outside.txt"), ReadKind::File).is_none());
    broker::remove_test_grant(&root_text);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = process.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "helper survived closure of its session"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Duplicated pins still deny rename after the helper session ends.
    assert!(std::fs::rename(&child_dir, root.join("moved-child")).is_err());
    assert!(std::fs::rename(&root, fixture.path().join("moved-root")).is_err());
    drop(child_file);
    drop(child_pin);
    drop(pinned);
    drop(child_pins);
    drop(denied_child);
    drop(denied_file);
    drop(denied);
    assert_eq!(std::fs::read(&source).unwrap(), b"protected payload");
}
