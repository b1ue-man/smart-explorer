use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) struct SignalServer {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SignalServer {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake Share signal server");
        listener
            .set_nonblocking(true)
            .expect("make fake Share signal server nonblocking");
        let address = listener.local_addr().expect("read fake signal address");
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_stop = stopped.clone();
        let thread = thread::spawn(move || serve(listener, thread_stop));
        Self {
            address,
            stopped,
            thread: Some(thread),
        }
    }

    pub(super) fn endpoint(&self) -> String {
        self.address.to_string()
    }
}

impl Drop for SignalServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("join fake Share signal server");
        }
    }
}

fn serve(listener: TcpListener, stopped: Arc<AtomicBool>) {
    let mut clients = Vec::new();
    while !stopped.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let client_stop = stopped.clone();
                clients.push(thread::spawn(move || drain(stream, client_stop)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("fake Share signal accept failed: {error}"),
        }
    }
    for client in clients {
        client.join().expect("join fake Share signal client");
    }
}

fn drain(mut stream: TcpStream, stopped: Arc<AtomicBool>) {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set fake signal read timeout");
    let mut buffer = [0_u8; 4096];
    while !stopped.load(Ordering::Relaxed) {
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}
