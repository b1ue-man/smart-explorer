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
    std::fs::write(fixture.path().join("outside.txt"), b"outside").unwrap();
    let denied = DeniedDirectory::new(&root);
    assert!(std::fs::read_dir(&root).is_err());
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
    drop(denied);
    assert_eq!(std::fs::read(&source).unwrap(), b"protected payload");
}
