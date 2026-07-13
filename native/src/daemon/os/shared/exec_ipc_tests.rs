use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use super::{read_frame, write_all_with_deadline, write_frame, INPUT_STDIN, MAX_DATA_BYTES};

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
