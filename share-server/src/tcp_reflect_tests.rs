use std::io;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

use super::*;

/// Reads until the server closes (or resets) the connection.
async fn read_to_end(stream: &TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 128];
    loop {
        if stream.readable().await.is_err() {
            return out;
        }
        match stream.try_read(&mut buffer) {
            Ok(0) => return out,
            Ok(read) => out.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return out,
        }
    }
}

async fn bounded<T>(work: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .expect("reflector test deadline")
}

/// User 2026-11-25 („TCP und UDP, DIREKT“): the reflector answers each
/// connection with the address it observed, holds it until its idle bound,
/// closes a connection beyond the per-source limit before any byte and
/// admits the source again once its place is free.
#[test]
fn tcp_reflect_task_answers_the_observed_address_and_holds_its_limits() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let limits = ReflectLimits {
            connections: ConnectionLimiter::new(8, 1),
            accepts: AcceptRateLimiter::new(),
            idle: Duration::from_millis(300),
        };
        let server = tokio::spawn(serve(listener, limits));

        let first = TcpStream::connect(address).await.unwrap();
        let expected = format!("se-reflect 1 {}\n", first.local_addr().unwrap());
        // One connection per source: the second closes without a byte.
        let second = TcpStream::connect(address).await.unwrap();
        assert!(bounded(read_to_end(&second)).await.is_empty());
        // The first gets its line and is closed after the idle bound.
        assert_eq!(bounded(read_to_end(&first)).await, expected.as_bytes());

        // Its place comes free once the server dropped it.
        let mut admitted = None;
        for _ in 0..50 {
            let third = TcpStream::connect(address).await.unwrap();
            let answer = bounded(read_to_end(&third)).await;
            if !answer.is_empty() {
                admitted = Some((answer, third.local_addr().unwrap()));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (answer, local) = admitted.expect("the source was admitted again");
        assert_eq!(answer, format!("se-reflect 1 {local}\n").as_bytes());
        server.abort();
    });

    // An IPv4-mapped source is named as IPv4.
    assert_eq!(
        line("[::ffff:203.0.113.7]:40123".parse().unwrap()),
        "se-reflect 1 203.0.113.7:40123\n"
    );
    assert_eq!(
        line("[2001:db8::9]:40123".parse().unwrap()),
        "se-reflect 1 [2001:db8::9]:40123\n"
    );
    assert_eq!(CAPABILITY, "tcp_reflect_v1");
    assert_eq!(DEFAULT_PORT, 51823);
}
