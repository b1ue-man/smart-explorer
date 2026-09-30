use super::{backend_server, ipc_protocol::MountBackendCapabilities, mount_proxy, rooted_backend::RootedBackend};
use crate::mount::{BackendRoot, MountMode, MountRootSecurity};
use crate::mount::range_read_task_tests::RangeBackend;
use crate::vfs::BackendHandle;
use std::{io::{self, Read}, net::{Shutdown, TcpListener, TcpStream}, sync::{Arc, atomic::Ordering}};

struct Bridge {
    backend: BackendHandle,
    stop: TcpStream,
    worker: Option<std::thread::JoinHandle<io::Result<()>>>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.stop.shutdown(Shutdown::Both);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn bridge(inner: BackendHandle) -> Bridge {
    let rooted = RootedBackend::new(inner.clone(), &BackendRoot::parse("/root").unwrap(),
        MountMode::ReadOnly, MountRootSecurity::Enforced).unwrap();
    let capabilities = MountBackendCapabilities::from_backend(&rooted);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let stop = client.try_clone().unwrap();
    mount_proxy::prepare_stream(&client).unwrap();
    let (server, _) = listener.accept().unwrap();
    mount_proxy::prepare_stream(&server).unwrap();
    let read = server.try_clone().unwrap();
    let worker = std::thread::spawn(move || backend_server::serve_backend(read, server, rooted));
    let agent = crate::agent::AgentBackend::from_streams(Box::new(client.try_clone().unwrap()),
        Box::new(client), inner).unwrap();
    Bridge { backend: mount_proxy::wrap(Arc::new(agent), capabilities), stop, worker: Some(worker) }
}

#[test]
fn mount_recovery_cache_task_range_crosses_rooted_tcp_proxy_without_ids_or_escape() {
    let raw = RangeBackend::new(450 * 1024 * 1024 * 1024);
    let bridge = bridge(raw.clone());
    for offset in [19, raw.size - 23] {
        let mut reader = bridge.backend.open_read_at("/large", Some("outside-provider-object"), offset)
            .unwrap().unwrap();
        let mut bytes = [0; 23];
        reader.read_exact(&mut bytes).unwrap();
        for (index, byte) in bytes.iter().enumerate() { assert_eq!(*byte, ((offset + index as u64) % 251) as u8); }
        drop(reader);
        // Releasing a partially consumed request must admit the next metadata
        // request and preserve the one connection's framing.
        assert_eq!(bridge.backend.stat("/large").unwrap().size, raw.size);
    }
    for path in ["/../large", "/link"] {
        // The agent may reject in its open handshake or while receiving data.
        // Both must preserve the original authorization error kind.
        let error = match bridge.backend.open_read_at(path, Some("private-provider-id"), 1) {
            Err(error) => error,
            Ok(Some(mut reader)) => reader.read(&mut [0; 1]).unwrap_err(),
            Ok(None) => panic!("the agent supports offsets and must reject this unauthorized path"),
        };
        assert!(matches!(error.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied));
        assert_eq!(bridge.backend.stat("/large").unwrap().size, raw.size);
    }
    assert_eq!(raw.full_reads.load(Ordering::SeqCst), 0);
    assert_eq!(raw.range_reads.load(Ordering::SeqCst), 2);
    assert!(raw.bytes.load(Ordering::SeqCst) < 16 * 1024 * 1024,
        "cancellation/credit must bound read-ahead for a tiny request");
}

#[test]
fn mount_recovery_cache_task_proxy_fallback_keeps_unseekable_backends_readable() {
    let raw = RangeBackend::new(2 * 1024 * 1024);
    raw.ranges.store(false, Ordering::SeqCst);
    let bridge = bridge(raw.clone());
    let mut reader = bridge.backend.open_read_at("/large", None, 123).unwrap().unwrap();
    let mut bytes = [0; 23];
    reader.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes[0], 123);
    assert_eq!(bytes[22], 145);
    drop(reader);
    assert_eq!(bridge.backend.stat("/large").unwrap().size, raw.size);
    assert_eq!(raw.full_reads.load(Ordering::SeqCst), 1);
}
