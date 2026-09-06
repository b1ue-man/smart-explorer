//! Test-only integration boundary using the actual production mount layers.
use super::{backend_server, ipc_protocol::MountBackendCapabilities, mount_proxy,
    rooted_backend::RootedBackend, vault_task_tree::VaultTree};
use crate::{mount::{BackendRoot, MountMode, MountRootSecurity,
    optimization_fixture::OptimizationBackend}, vfs::{Backend, BackendHandle}};
use std::{io, net::{Shutdown, TcpListener, TcpStream}, sync::{mpsc, Arc},
    thread::JoinHandle, time::Duration};
pub(crate) use super::vault_task_tree::VaultTaskCounters;

pub(crate) struct VaultTaskBridge {
    pub backend: BackendHandle,
    pub source: Arc<OptimizationBackend>,
    tree: Arc<VaultTree>,
    shutdown: TcpStream,
    worker: Option<JoinHandle<io::Result<()>>>,
    finished: mpsc::Receiver<()>,
}

impl VaultTaskBridge {
    pub fn new() -> io::Result<Self> {
        let tree = VaultTree::new();
        let raw: BackendHandle = tree.clone();
        let rooted = RootedBackend::new(raw.clone(), &BackendRoot::parse("/")?,
            MountMode::ReadWrite, MountRootSecurity::Enforced)?;
        let capabilities = MountBackendCapabilities::from_backend(&rooted);
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let client = TcpStream::connect(listener.local_addr()?)?;
        let (server, _) = listener.accept()?;
        mount_proxy::prepare_stream(&client)?;
        mount_proxy::prepare_stream(&server)?;
        assert!(client.nodelay()? && server.nodelay()?, "mount endpoints need TCP_NODELAY");
        let shutdown = client.try_clone()?;
        let client_read = client.try_clone()?;
        let read = server.try_clone()?;
        let (done, finished) = mpsc::channel();
        let worker = std::thread::Builder::new().name("vault-task-daemon-backend".into())
            .spawn(move || {
                let result = backend_server::serve_backend(read, server, rooted);
                let _ = done.send(());
                result
            })?;
        let agent = crate::agent::AgentBackend::from_streams(
            Box::new(client_read), Box::new(client), raw);
        let agent = match agent {
            Ok(agent) => agent,
            Err(error) => {
                let _ = shutdown.shutdown(Shutdown::Both);
                await_shutdown(&finished);
                let _ = worker.join();
                return Err(error);
            }
        };
        Ok(Self { backend: mount_proxy::wrap(Arc::new(agent), capabilities),
            source: tree.source.clone(), tree, shutdown, worker: Some(worker), finished })
    }

    pub fn counters(&self) -> VaultTaskCounters { self.tree.counters() }

    pub fn finish(&mut self) -> io::Result<()> {
        let Some(worker) = self.worker.take() else { return Ok(()); };
        let _ = self.shutdown.shutdown(Shutdown::Both);
        await_shutdown(&self.finished);
        worker.join().map_err(|_| io::Error::other("vault daemon worker panicked"))?
    }
}

fn await_shutdown(finished: &mpsc::Receiver<()>) {
    if matches!(finished.recv_timeout(Duration::from_secs(15)), Err(mpsc::RecvTimeoutError::Timeout)) {
        // Never continue another runtime with an unjoined daemon request. This
        // also bounds the standalone leaf-refresh case, outside volume timers.
        eprintln!("[mount vault] fatal daemon shutdown deadline after TCP shutdown");
        std::process::abort();
    }
}

impl Drop for VaultTaskBridge {
    fn drop(&mut self) { let _ = self.finish(); }
}

#[test]
fn mount_vault_task_rooted_refresh_crosses_daemon_ttl() -> io::Result<()> {
    let mut bridge = VaultTaskBridge::new()?;
    bridge.source.mkdir("/external");
    assert!(bridge.backend.list_dir("/external")?.is_empty());
    bridge.source.put("/external/new.md", b"new");
    let listed = bridge.backend.list_dir("/external")?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "new.md");
    assert_eq!(bridge.backend.stat("/EXTERNAL/NEW.md")?.size, 3);
    bridge.source.remove_file("/external/new.md")?;
    assert!(bridge.backend.list_dir("/external")?.is_empty());
    assert_eq!(bridge.backend.stat("/external/new.md").unwrap_err().kind(), io::ErrorKind::NotFound);
    bridge.finish()
}

#[test]
fn mount_vault_task_framed_tcp_latency_diagnostic() -> io::Result<()> {
    use crate::agent_proto::{read_frame, write_frame, Frame};
    use std::{io::Write, time::{Duration, Instant}};
    fn send(stream: &mut TcpStream, id: u64, frame: &Frame, complete: bool) -> io::Result<()> {
        if complete { return write_frame(stream, id, frame); }
        let body = frame.encode(id)?;
        stream.write_all(&(body.len() as u32).to_le_bytes())?;
        stream.write_all(&body)?;
        stream.flush()
    }
    for complete in [false, true] {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let mut client = TcpStream::connect(listener.local_addr()?)?;
        let (mut server, _) = listener.accept()?;
        for stream in [&client, &server] {
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.set_write_timeout(Some(Duration::from_secs(5)))?;
            if complete { mount_proxy::prepare_stream(stream)?; }
            assert_eq!(stream.nodelay()?, complete);
        }
        std::thread::scope(|scope| -> io::Result<()> {
            let echo = scope.spawn(move || -> io::Result<()> {
                for id in 0..32 {
                    assert_eq!(read_frame(&mut server)?, Some((id, Frame::Stat("/note.md".into()))));
                    send(&mut server, id, &Frame::Ok, complete)?;
                }
                Ok(())
            });
            let started = Instant::now();
            let result = (|| -> io::Result<()> {
                for id in 0..32 {
                    send(&mut client, id, &Frame::Stat("/note.md".into()), complete)?;
                    assert_eq!(read_frame(&mut client)?, Some((id, Frame::Ok)));
                }
                Ok(())
            })();
            eprintln!("[mount vault] controlled_loopback complete_frame_and_nodelay={complete} round_trips=32 elapsed_us={}",
                started.elapsed().as_micros());
            let _ = client.shutdown(Shutdown::Both);
            let peer = echo.join().map_err(|_| io::Error::other("loopback diagnostic panicked"))?;
            result.and(peer)
        })?;
    }
    Ok(())
}
