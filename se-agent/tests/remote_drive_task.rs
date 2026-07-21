#![cfg(target_os = "linux")]

#[path = "../../native/src/agent_proto/mod.rs"]
mod agent_proto;

use agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> io::Result<Self> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "smart-explorer-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spawn_confined(root: &Path) -> io::Result<(Child, ChildStdin, ChildStdout)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_se-agent"))
        .arg("--serve-root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("agent stdin was not piped"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("agent stdout was not piped"))?;
    Ok((child, stdin, stdout))
}

fn response(stdout: &mut ChildStdout, expected_id: u64) -> Frame {
    let (id, frame) = read_frame(stdout)
        .expect("agent response frame should be readable")
        .expect("agent should not close stdout before responding");
    assert_eq!(id, expected_id, "agent response used the wrong request id");
    frame
}

fn read_request(stdin: &mut ChildStdin, stdout: &mut ChildStdout, id: u64, path: &Path) -> Vec<u8> {
    write_frame(
        stdin,
        id,
        &Frame::Read {
            path: path.to_string_lossy().into_owned(),
            offset: 0,
            len: 0,
        },
    )
    .expect("read request should be written");
    let mut bytes = Vec::new();
    loop {
        match response(stdout, id) {
            Frame::Data(chunk) => bytes.extend_from_slice(&chunk),
            Frame::End => return bytes,
            Frame::Err(error) => panic!("agent read failed unexpectedly: {error}"),
            frame => panic!("unexpected agent read response: {frame:?}"),
        }
    }
}

fn expect_error(stdout: &mut ChildStdout, id: u64) -> String {
    match response(stdout, id) {
        Frame::Err(error) => error,
        frame => panic!("expected agent error, received {frame:?}"),
    }
}

#[test]
fn remote_drive_task_agent_root_is_kernel_confined_and_write_new_is_exclusive() {
    let parent = TestDirectory::new("agent-root").expect("create test root");
    let root = parent.path().join("allowed");
    std::fs::create_dir(&root).expect("create allowed root");
    let inside = root.join("inside.txt");
    let outside = parent.path().join("outside.txt");
    std::fs::write(&inside, b"inside").expect("seed in-root file");
    std::fs::write(&outside, b"outside").expect("seed outside file");

    let (mut child, mut stdin, mut stdout) =
        spawn_confined(&root).expect("start root-confined agent");
    write_frame(
        &mut stdin,
        1,
        &Frame::Hello {
            proto: PROTO_VERSION,
        },
    )
    .expect("write hello");
    assert!(matches!(
        response(&mut stdout, 1),
        Frame::HelloOk {
            proto: PROTO_VERSION,
            ..
        }
    ));

    assert_eq!(read_request(&mut stdin, &mut stdout, 2, &inside), b"inside");
    write_frame(
        &mut stdin,
        3,
        &Frame::Read {
            path: outside.to_string_lossy().into_owned(),
            offset: 0,
            len: 0,
        },
    )
    .expect("write outside read request");
    let _ = expect_error(&mut stdout, 3);

    let created = root.join("created.txt");
    write_frame(
        &mut stdin,
        4,
        &Frame::WriteNew(created.to_string_lossy().into_owned()),
    )
    .expect("write exclusive-create request");
    assert!(matches!(
        response(&mut stdout, 4),
        Frame::Progress { done: 0, total: 0 }
    ));
    write_frame(&mut stdin, 4, &Frame::Data(b"created".to_vec())).expect("write data");
    write_frame(&mut stdin, 4, &Frame::End).expect("finish write");
    assert_eq!(response(&mut stdout, 4), Frame::Ok);

    write_frame(
        &mut stdin,
        5,
        &Frame::WriteNew(created.to_string_lossy().into_owned()),
    )
    .expect("write colliding exclusive-create request");
    let _ = expect_error(&mut stdout, 5);
    assert_eq!(
        std::fs::read(&created).expect("read created file"),
        b"created"
    );
    assert_eq!(
        std::fs::read(&outside).expect("read outside file"),
        b"outside"
    );

    drop(stdin);
    drop(stdout);
    let status = child.wait().expect("wait for confined agent");
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        panic!("confined agent failed after clean EOF: {status}: {stderr}");
    }
}

#[test]
fn remote_drive_task_agent_rejects_a_symlinked_root_before_serving() {
    use std::os::unix::fs::symlink;

    let parent = TestDirectory::new("agent-symlink-root").expect("create test root");
    let actual = parent.path().join("actual");
    let linked = parent.path().join("linked");
    std::fs::create_dir(&actual).expect("create actual root");
    symlink(&actual, &linked).expect("create root symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_se-agent"))
        .arg("--serve-root")
        .arg(&linked)
        .stdin(Stdio::null())
        .output()
        .expect("run agent against symlinked root");
    assert!(!output.status.success(), "symlinked root was accepted");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("secure root confinement unavailable"),
        "unexpected confinement error: {stderr}"
    );
}
