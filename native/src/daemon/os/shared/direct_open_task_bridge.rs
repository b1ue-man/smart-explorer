//! Isolated test bridge through the real daemon framing and AgentBackend.
use std::io;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub(crate) struct DirectOpenTaskBridge {
    pub(crate) backend: crate::vfs::BackendHandle,
    shutdown: TcpStream,
    worker: Option<std::thread::JoinHandle<io::Result<()>>>,
    done: mpsc::Receiver<()>,
}

impl DirectOpenTaskBridge {
    pub(crate) fn new(source: crate::vfs::BackendHandle) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let client = TcpStream::connect(listener.local_addr()?)?;
        let (server, _) = listener.accept()?;
        client.set_nodelay(true)?;
        server.set_nodelay(true)?;
        let shutdown = client.try_clone()?;
        let reader = client.try_clone()?;
        let server_reader = server.try_clone()?;
        let (tx, done) = mpsc::channel();
        let backend = source.clone();
        let worker = std::thread::spawn(move || {
            let result = super::backend_server::serve_backend(server_reader, server, backend);
            let _ = tx.send(());
            result
        });
        let agent = crate::agent::AgentBackend::from_streams(
            Box::new(reader), Box::new(client), source,
        );
        match agent {
            Ok(agent) => Ok(Self { backend: Arc::new(agent), shutdown,
                worker: Some(worker), done }),
            Err(error) => {
                let _ = shutdown.shutdown(Shutdown::Both);
                done.recv_timeout(Duration::from_secs(15)).expect("bridge shutdown");
                let _ = worker.join();
                Err(error)
            }
        }
    }
}

impl Drop for DirectOpenTaskBridge {
    fn drop(&mut self) {
        let _ = self.shutdown.shutdown(Shutdown::Both);
        self.done.recv_timeout(Duration::from_secs(15)).expect("bridge shutdown deadline");
        if let Some(worker) = self.worker.take() {
            // Closing the socket may surface EOF or a platform socket error.
            let _ = worker.join().expect("bridge worker panicked");
        }
    }
}
