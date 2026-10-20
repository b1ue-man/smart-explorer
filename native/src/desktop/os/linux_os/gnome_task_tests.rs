//! DSK-M8 signals about the channel and the files between the shell and the
//! extension: `desktop_task_gnome_channel_rejects_wrong_uid_and_token` and
//! `desktop_task_gnome_control_files_atomic`.
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::gnome_channel::{peer_credentials, surface_socket, GnomeListener, ListenerEvent};
use super::gnome_control::{Control, ControlMonitor, RuntimeFiles, PROTOCOL};
use crate::desktop::core::protocol::{write_frame, SurfaceMessage, PROTOCOL_VERSION};

fn own_uid() -> u32 {
    // SAFETY: `getuid` has no arguments and cannot fail.
    unsafe { libc::getuid() }
}

fn runtime(root: &Path) -> RuntimeFiles {
    RuntimeFiles::at(root.join("run").join("smart-explorer"))
}

fn hello(monitor: &str, token: Option<&str>) -> SurfaceMessage {
    SurfaceMessage::Hello {
        protocol: PROTOCOL_VERSION,
        monitor: monitor.into(),
        pid: 99,
        token: token.map(str::to_string),
    }
}

/// The next event of the listener, up to two seconds.
fn next_event(listener: &GnomeListener) -> Option<ListenerEvent> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(event) = listener.drain().into_iter().next() {
            return Some(event);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// The connection is closed by the listener (end of stream).
fn closed(stream: &mut UnixStream) -> bool {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut byte = [0u8; 1];
    matches!(stream.read(&mut byte), Ok(0))
}

#[test]
fn desktop_task_gnome_channel_rejects_wrong_uid_and_token() {
    let root = tempfile::tempdir().unwrap();
    let files = runtime(root.path());
    let token = files.new_token().unwrap();
    let wait = Duration::from_millis(400);
    let same_user: Arc<super::gnome_channel::PeerFn> = Arc::new(|_| Ok((own_uid(), 4242)));

    // The right user, the right token, a GNOME monitor: accepted.
    let listener = GnomeListener::bind_with(&files, same_user.clone(), wait).unwrap();
    let mut good = UnixStream::connect(files.socket_path()).unwrap();
    write_frame(&mut good, &hello("gnome-3", Some(&token))).unwrap();
    match next_event(&listener) {
        Some(ListenerEvent::Accepted(accepted)) => {
            assert_eq!((accepted.index, accepted.pid), (3, 4242));
        }
        other => panic!("accepted expected: {other:?}"),
    }
    // The socket belongs to the user alone.
    let mode = std::fs::metadata(files.socket_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");

    // A wrong token, none, a wrong protocol, a monitor that is no GNOME
    // monitor, no greeting at all: refused and closed.
    let refuse = |frame: Option<SurfaceMessage>, why: &str| {
        let mut stream = UnixStream::connect(files.socket_path()).unwrap();
        if let Some(frame) = frame {
            write_frame(&mut stream, &frame).unwrap();
        }
        match next_event(&listener) {
            Some(ListenerEvent::Rejected(reason)) => {
                assert!(reason.contains(why), "{reason:?} should name {why:?}");
            }
            other => panic!("rejection expected: {other:?}"),
        }
        assert!(closed(&mut stream), "closed after: {why}");
    };
    refuse(
        Some(hello("gnome-0", Some("0".repeat(32).as_str()))),
        "Token",
    );
    refuse(Some(hello("gnome-0", None)), "Token");
    let mut old = hello("gnome-0", Some(&token));
    if let SurfaceMessage::Hello { protocol, .. } = &mut old {
        *protocol = 2;
    }
    refuse(Some(old), "Protokoll");
    refuse(Some(hello("layer", Some(&token))), "kein GNOME-Monitor");
    refuse(Some(SurfaceMessage::Alive), "keine Begrüßung");
    refuse(None, "keine Begrüßung");

    // Another user (the check of `SO_PEERCRED`): closed before any frame.
    let other_user: Arc<super::gnome_channel::PeerFn> =
        Arc::new(|_| Ok((own_uid().wrapping_add(1), 1)));
    let strict = GnomeListener::bind_with(&files, other_user, wait).unwrap();
    let mut stranger = UnixStream::connect(files.socket_path()).unwrap();
    let _ = write_frame(&mut stranger, &hello("gnome-0", Some(&token)));
    match next_event(&strict) {
        Some(ListenerEvent::Rejected(reason)) => assert!(reason.contains("fremder Benutzer")),
        other => panic!("rejection expected: {other:?}"),
    }
    assert!(closed(&mut stranger));

    // The real call: a socket pair belongs to this process and this user.
    let (left, _right) = UnixStream::pair().unwrap();
    assert_eq!(
        peer_credentials(&left).unwrap(),
        (own_uid(), std::process::id())
    );
    // The surface's end needs a token before it connects to anything.
    assert!(surface_socket(&files.socket_path(), "", || {}).is_err());
}

fn monitors_of(count: usize) -> Vec<ControlMonitor> {
    (0..count)
        .map(|index| ControlMonitor {
            index: index as u32,
            key: format!("DP-{index}"),
            x: (index as i32) * 1920,
            y: 0,
            width: 1920,
            height: 1080,
        })
        .collect()
}

#[test]
fn desktop_task_gnome_control_files_atomic() {
    let root = tempfile::tempdir().unwrap();
    let files = runtime(root.path());
    let control = |generation: u64, count: usize| Control {
        protocol: PROTOCOL,
        exe: "/opt/se/smart_explorer".into(),
        generation,
        socket: files.socket_path().to_string_lossy().into_owned(),
        monitors: monitors_of(count),
    };
    files.write_control(&control(1, 1)).unwrap();
    // Folder 0700, file 0600.
    let dir_mode = std::fs::metadata(files.dir()).unwrap().permissions().mode();
    let file_mode = std::fs::metadata(files.control_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o077, 0, "{dir_mode:o}");
    assert_eq!(file_mode & 0o077, 0, "{file_mode:o}");

    // A reader never sees half a file while the writer replaces it.
    let path = files.control_path();
    let reader = std::thread::spawn(move || {
        let mut seen = 0;
        let deadline = Instant::now() + Duration::from_secs(20);
        while seen < 3000 && Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let parsed = serde_json::from_str::<Control>(&text);
                assert!(parsed.is_ok(), "half a file: {} bytes", text.len());
                seen += 1;
            }
        }
        seen
    });
    for generation in 2..400u64 {
        let count = if generation % 2 == 0 { 400 } else { 1 };
        files.write_control(&control(generation, count)).unwrap();
        if reader.is_finished() {
            break;
        }
    }
    let last = control(1000, 2);
    files.write_control(&last).unwrap();
    assert!(reader.join().unwrap() > 0, "the reader saw the file");
    assert_eq!(files.read_control(), Some(last.clone()));
    // No helper file stays.
    let leftovers: Vec<_> = std::fs::read_dir(files.dir())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "desktop-gnome.json")
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // The JSON the extension reads.
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(files.control_path()).unwrap()).unwrap();
    for key in ["protocol", "exe", "generation", "socket", "monitors"] {
        assert!(value.get(key).is_some(), "{key}");
    }
    assert_eq!(value["monitors"][1]["index"], 1);
    assert_eq!(value["monitors"][1]["key"], "DP-1");
    assert_eq!(value["monitors"][1]["x"], 1920);

    // The token: 32 hex digits, 0600, new at every start.
    let first = files.new_token().unwrap();
    assert_eq!(first.len(), 32);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(files.read_token().as_deref(), Some(first.as_str()));
    let token_mode = std::fs::metadata(files.token_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(token_mode & 0o077, 0);
    let second = files.new_token().unwrap();
    assert_ne!(first, second);
    // What a surface needs.
    let (socket, token) = files.surface_side().unwrap();
    assert_eq!(socket, files.socket_path());
    assert_eq!(token, second);

    // The report of the extension (written by its JavaScript).
    std::fs::write(
        files.extension_path(),
        r#"{"version":1,"variant":"a","surfaces":[{"monitor":0,"pid":123,"handled":true}],"halted":null}"#,
    )
    .unwrap();
    let report = files.read_extension().unwrap();
    assert!(report.handles_a_window() && report.halted.is_none());
    std::fs::write(
        files.extension_path(),
        r#"{"version":1,"variant":"b","surfaces":[],"halted":{"reason":"crash_loop","at":1760000000}}"#,
    )
    .unwrap();
    let halted = files.read_extension().unwrap().halted.unwrap();
    assert_eq!(
        (halted.reason.as_str(), halted.at),
        ("crash_loop", 1_760_000_000)
    );
    std::fs::write(files.extension_path(), "{ half").unwrap();
    assert!(files.read_extension().is_none());

    // The shell's end removes control file, token and socket, not the
    // extension's report (the extension deletes that itself).
    let _socket = UnixListener::bind(files.socket_path()).unwrap();
    files.remove_shell_files();
    for path in [
        files.control_path(),
        files.token_path(),
        files.socket_path(),
    ] {
        assert!(!path.exists(), "{}", path.display());
    }
    assert!(files.extension_path().exists());
    assert!(files.surface_side().is_err());
}
