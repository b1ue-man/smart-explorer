use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    client_input, read_frame, write_all_with_deadline, write_frame, ExecIpcInput, INPUT_CANCEL,
    INPUT_EOF, INPUT_RESIZE, INPUT_STDIN, MAX_DATA_BYTES,
};
use crate::share::{ExecClientInput, ExecTerminalSize};

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let client = TcpStream::connect(address).unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

#[test]
fn local_exec_frames_preserve_arbitrary_binary_data() {
    let (mut client, mut server) = pair();
    let payload = vec![0, 1, 0xff, b'\n', 0, 0x80];
    write_frame(&mut client, INPUT_STDIN, &payload).unwrap();
    assert_eq!(
        read_frame(&mut server, MAX_DATA_BYTES).unwrap(),
        (INPUT_STDIN, payload)
    );
}

#[test]
fn oversized_local_exec_frame_is_rejected_before_payload_read() {
    let (mut client, mut server) = pair();
    client.write_all(&[INPUT_STDIN]).unwrap();
    client
        .write_all(&((MAX_DATA_BYTES as u32) + 1).to_be_bytes())
        .unwrap();
    let error = read_frame(&mut server, MAX_DATA_BYTES).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn trickle_progress_cannot_extend_the_absolute_local_write_deadline() {
    struct TrickleWriter(usize);

    impl Write for TrickleWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(Duration::from_millis(5));
            self.0 += 1;
            Ok(bytes.len().min(1))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let started = Instant::now();
    let mut writer = TrickleWriter(0);
    let error = write_all_with_deadline(
        &mut writer,
        &[1; 64],
        started + Duration::from_millis(30),
        |_, _| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(writer.0 < 64);
}

#[test]
fn terminal_task_resize_frame_becomes_a_client_resize_input() {
    let (client, mut server) = pair();
    let (control, _control_peer) = pair();
    let input = ExecIpcInput {
        stream: Arc::new(Mutex::new(client)),
        control: Arc::new(control),
    };
    let size = ExecTerminalSize {
        cols: 132,
        rows: 43,
    };
    input.resize(size).unwrap();
    let (tag, payload) = read_frame(&mut server, MAX_DATA_BYTES).unwrap();
    assert_eq!(tag, INPUT_RESIZE);
    assert_eq!(tag, 4);
    assert_eq!(payload, br#"{"cols":132,"rows":43}"#);
    assert_eq!(
        client_input(tag, payload).unwrap(),
        ExecClientInput::Resize(size)
    );
}

#[test]
fn terminal_task_unusable_or_malformed_resize_frames_are_refused() {
    let (client, mut server) = pair();
    let (control, _control_peer) = pair();
    let input = ExecIpcInput {
        stream: Arc::new(Mutex::new(client)),
        control: Arc::new(control),
    };
    let error = input
        .resize(ExecTerminalSize { cols: 0, rows: 24 })
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    // Nothing was written for the refused size.
    server
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    assert!(read_frame(&mut server, MAX_DATA_BYTES).is_err());

    let oversized = format!("{{\"cols\":80,\"rows\":24{}}}", " ".repeat(300)).into_bytes();
    for payload in [
        br#"{"cols":0,"rows":24}"#.to_vec(),
        br#"{"cols":80,"rows":32768}"#.to_vec(),
        br#"{"cols":-1,"rows":24}"#.to_vec(),
        b"not json".to_vec(),
        Vec::new(),
        oversized,
    ] {
        let error = client_input(INPUT_RESIZE, payload).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "invalid local Exec input frame");
    }
}

#[test]
fn terminal_task_existing_input_frames_keep_their_meaning() {
    assert_eq!(
        client_input(INPUT_STDIN, vec![0, 0xff]).unwrap(),
        ExecClientInput::Stdin(vec![0, 0xff])
    );
    assert_eq!(
        client_input(INPUT_EOF, Vec::new()).unwrap(),
        ExecClientInput::StdinEof
    );
    assert_eq!(
        client_input(INPUT_CANCEL, Vec::new()).unwrap(),
        ExecClientInput::Cancel
    );
    assert!(client_input(INPUT_EOF, vec![1]).is_err());
    assert!(client_input(INPUT_CANCEL, vec![1]).is_err());
    assert!(client_input(99, Vec::new()).is_err());
}
