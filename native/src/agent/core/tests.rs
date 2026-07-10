use super::*;
use crate::vfs::Backend;
use crossbeam_channel::unbounded;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn agent_with_walk_reply(
    reply: Option<crate::agent_proto::Frame>,
) -> (AgentBackend, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut read = socket.try_clone().unwrap();
        let (hello_id, hello) = crate::agent_proto::read_frame(&mut read)
            .unwrap()
            .expect("hello frame");
        assert!(matches!(hello, crate::agent_proto::Frame::Hello { .. }));
        crate::agent_proto::write_frame(
            &mut socket,
            hello_id,
            &crate::agent_proto::Frame::HelloOk {
                proto: crate::agent_proto::PROTO_VERSION,
                version: "test".into(),
            },
        )
        .unwrap();
        let (walk_id, walk) = crate::agent_proto::read_frame(&mut read)
            .unwrap()
            .expect("walk frame");
        assert!(matches!(walk, crate::agent_proto::Frame::WalkTree(_)));
        if let Some(reply) = reply {
            crate::agent_proto::write_frame(&mut socket, walk_id, &reply).unwrap();
        }
    });

    let client = TcpStream::connect(addr).unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let inner: crate::vfs::BackendHandle = std::sync::Arc::new(crate::vfs::LocalBackend::new("/"));
    (
        AgentBackend::from_streams(read, write, inner).unwrap(),
        server,
    )
}

#[test]
fn walk_tree_surfaces_remote_and_disconnected_transport_errors() {
    let (remote_error, server) =
        agent_with_walk_reply(Some(crate::agent_proto::Frame::Err("walk denied".into())));
    let error = remote_error.walk_tree("/", &|_, _| true).unwrap_err();
    assert!(error.to_string().contains("walk denied"));
    drop(remote_error);
    server.join().unwrap();

    let (disconnected, server) = agent_with_walk_reply(None);
    let error = disconnected.walk_tree("/", &|_, _| true).unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::BrokenPipe
    ));
    drop(disconnected);
    server.join().unwrap();

    let (canceled, server) = agent_with_walk_reply(Some(crate::agent_proto::Frame::Progress {
        done: 1,
        total: 2,
    }));
    let error = canceled.walk_tree("/", &|_, _| false).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    drop(canceled);
    server.join().unwrap();
}

#[test]
fn artifact_selection_and_quoting() {
    let a = artifact_for("Linux x86_64").expect("x86_64 bundled");
    assert!(a.bytes.len() > 1000 && a.bytes.starts_with(b"\x7fELF"));
    assert!(artifact_for("Linux aarch64").is_some());
    assert!(artifact_for("Darwin arm64").is_none());
    assert!(artifact_for("garbage").is_none());
    assert_eq!(super::deploy::sha256_hex(a.bytes).len(), 64);
    assert_eq!(super::deploy::sh_quote("/home/u/dir"), "'/home/u/dir'");
    assert_eq!(
        super::deploy::sh_quote("a'b; rm -rf /"),
        r#"'a'\''b; rm -rf /'"#
    );
}

#[test]
fn agent_backend_over_socket() {
    let base = std::env::temp_dir().join(format!("se_agbe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sub")).unwrap();
    std::fs::write(base.join("a.txt"), vec![7u8; 100]).unwrap();
    std::fs::write(base.join("sub/b.bin"), vec![0u8; 400]).unwrap();
    let root = base.to_string_lossy().to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        let r = sock.try_clone().unwrap();
        let _ = crate::agent_proto::serve(r, sock);
    });

    let client = TcpStream::connect(addr).unwrap();
    let shut = client.try_clone().unwrap();
    let r: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let w: Box<dyn Write + Send> = Box::new(client);
    let inner: crate::vfs::BackendHandle = std::sync::Arc::new(crate::vfs::LocalBackend::new("/"));
    let be = AgentBackend::from_streams(r, w, inner).unwrap();

    let mut entries = be.list_dir(&root).unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries.iter().find(|e| e.name == "a.txt").unwrap().size,
        100
    );
    assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

    assert!(be.supports_walk_tree());
    let tree = crate::analytics::from_wire(
        be.walk_tree(&root, &|_, _| true)
            .unwrap()
            .expect("agent tree"),
    );
    assert_eq!(tree.size, 500);
    assert_eq!(
        tree.children
            .iter()
            .find(|c| &*c.name == "sub")
            .unwrap()
            .size,
        400
    );
    assert!(be
        .walk_tree(&format!("{root}/missing"), &|_, _| true)
        .is_err());

    let m = be.stat(&format!("{}/a.txt", root)).unwrap();
    assert_eq!(m.size, 100);
    assert!(!m.is_dir);

    let mut buf = Vec::new();
    be.open_read(&format!("{}/a.txt", root))
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();
    assert_eq!(buf, vec![7u8; 100]);

    {
        let (stx, srx) = unbounded();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let spec = crate::agent_proto::SearchSpec {
            query: "b".into(),
            glob: false,
            min_size: 0,
            max_size: 0,
            max_results: 0,
            want_dirs: false,
        };
        assert!(be.search(&root, &spec, stx, &cancel).unwrap());
        let hits: Vec<String> = srx.iter().map(|h| h.rel).collect();
        assert_eq!(hits, vec!["sub/b.bin".to_string()]);
    }

    {
        let (htx, hrx) = unbounded();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert!(be.walk_hashed(&root, true, htx, &cancel).unwrap());
        let hits: Vec<crate::vfs::HashHit> = hrx.iter().collect();
        let files: Vec<&crate::vfs::HashHit> = hits.iter().filter(|h| !h.is_dir).collect();
        assert_eq!(files.len(), 2);
        assert!(files
            .iter()
            .all(|h| h.md5.as_ref().is_some_and(|m| m.len() == 32)));
        let bbin = files.iter().find(|h| h.rel == "sub/b.bin").unwrap();
        assert_eq!(bbin.size, 400);
        let expect = format!("{:x}", md5::compute(vec![0u8; 400]));
        assert_eq!(bbin.md5.as_deref(), Some(expect.as_str()));
    }

    {
        let mut w = be.open_write(&format!("{}/written.dat", root)).unwrap();
        w.write_all(b"hello agent write").unwrap();
        w.flush().unwrap();
    }
    assert_eq!(
        std::fs::read(base.join("written.dat")).unwrap(),
        b"hello agent write"
    );

    std::fs::write(base.join("aborted.dat"), b"keep existing").unwrap();
    {
        let mut writer = be.open_write(&format!("{}/aborted.dat", root)).unwrap();
        writer.write_all(b"partial replacement").unwrap();
        // No flush: dropping must cancel and preserve the old destination.
    }
    let staged_exists = || {
        std::fs::read_dir(&base).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("aborted.dat.se-agent-")
        })
    };
    for _ in 0..50 {
        if !staged_exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(base.join("aborted.dat")).unwrap(),
        b"keep existing"
    );
    assert!(!staged_exists());

    be.mkdir_all(&format!("{}/newdir/inner", root)).unwrap();
    assert!(base.join("newdir/inner").is_dir());
    be.copy_file(
        &format!("{}/a.txt", root),
        &format!("{}/newdir/copy.txt", root),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(base.join("newdir/copy.txt")).unwrap().len(),
        100
    );
    be.rename(
        &format!("{}/newdir/copy.txt", root),
        &format!("{}/newdir/moved.txt", root),
    )
    .unwrap();
    assert!(!base.join("newdir/copy.txt").exists() && base.join("newdir/moved.txt").exists());
    be.remove_file(&format!("{}/newdir/moved.txt", root))
        .unwrap();
    assert!(!base.join("newdir/moved.txt").exists());
    assert!(be.remove_dir(&format!("{}/newdir", root)).is_err());

    assert!(be.supports_bulk_tree());
    let upsrc = base.join("upsrc");
    std::fs::create_dir_all(upsrc.join("sub")).unwrap();
    std::fs::write(upsrc.join("f1.txt"), b"one").unwrap();
    std::fs::write(upsrc.join("sub/f2.txt"), b"two longer").unwrap();
    let remote_dst = format!("{}/uploaded", root);
    std::fs::create_dir_all(base.join("uploaded")).unwrap();
    std::fs::write(base.join("uploaded/f1.txt"), b"old destination").unwrap();
    assert_eq!(be.put_tree(&upsrc, &remote_dst).unwrap(), 2);
    assert_eq!(std::fs::read(base.join("uploaded/f1.txt")).unwrap(), b"one");
    assert_eq!(
        std::fs::read(base.join("uploaded/sub/f2.txt")).unwrap(),
        b"two longer"
    );
    let getdst = base.join("downloaded");
    std::fs::create_dir_all(&getdst).unwrap();
    std::fs::write(getdst.join("f1.txt"), b"old destination").unwrap();
    assert_eq!(be.get_tree(&remote_dst, &getdst).unwrap(), 2);
    assert_eq!(std::fs::read(getdst.join("f1.txt")).unwrap(), b"one");
    assert_eq!(
        std::fs::read(getdst.join("sub/f2.txt")).unwrap(),
        b"two longer"
    );

    drop(be);
    let _ = shut.shutdown(std::net::Shutdown::Both);
    let _ = server.join();
    let _ = std::fs::remove_dir_all(&base);
}

struct UnavailableInner;

impl crate::vfs::Backend for UnavailableInner {
    fn scheme(&self) -> crate::vfs::Scheme {
        crate::vfs::Scheme::Peer
    }

    fn root_display(&self) -> String {
        "unavailable".into()
    }

    fn list_dir(&self, _path: &str) -> crate::vfs::VfsResult<Vec<crate::vfs::VfsMeta>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }

    fn stat(&self, _path: &str) -> crate::vfs::VfsResult<crate::vfs::VfsMeta> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }

    fn open_read(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Read + Send>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }

    fn open_write(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Write + Send>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }

    fn rename(&self, _src: &str, _dst: &str) -> crate::vfs::VfsResult<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }

    fn remove_file(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }

    fn remove_dir(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }

    fn mkdir_all(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "inner unavailable",
        ))
    }
}

#[test]
fn probes_and_no_replace_do_not_delegate_to_unavailable_inner() {
    let base = std::env::temp_dir().join(format!(
        "se_agent_atomic_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("present.txt"), b"present").unwrap();
    std::fs::write(base.join("source.txt"), b"source").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        crate::agent_proto::serve(read, socket).unwrap();
    });
    let client = TcpStream::connect(addr).unwrap();
    let shutdown = client.try_clone().unwrap();
    let backend = AgentBackend::from_streams(
        Box::new(client.try_clone().unwrap()),
        Box::new(client),
        std::sync::Arc::new(UnavailableInner),
    )
    .unwrap();

    assert!(backend
        .try_exists(base.join("present.txt").to_str().unwrap())
        .unwrap());
    assert!(!backend
        .try_exists(base.join("missing.txt").to_str().unwrap())
        .unwrap());
    backend
        .rename_no_replace(
            base.join("source.txt").to_str().unwrap(),
            base.join("moved.txt").to_str().unwrap(),
        )
        .unwrap();
    assert_eq!(std::fs::read(base.join("moved.txt")).unwrap(), b"source");

    std::fs::write(base.join("second.txt"), b"second").unwrap();
    std::fs::write(base.join("occupied.txt"), b"occupied").unwrap();
    assert!(backend
        .rename_no_replace(
            base.join("second.txt").to_str().unwrap(),
            base.join("occupied.txt").to_str().unwrap(),
        )
        .is_err());
    assert_eq!(std::fs::read(base.join("second.txt")).unwrap(), b"second");
    assert_eq!(
        std::fs::read(base.join("occupied.txt")).unwrap(),
        b"occupied"
    );

    drop(backend);
    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    let _ = server.join();
    let _ = std::fs::remove_dir_all(base);
}

#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn real_agent_binary_child_process() {
    use std::process::{Command, Stdio};
    let bin = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/agent-bin/se-agent-x86_64-linux-musl"
    );
    let base = std::env::temp_dir().join(format!("se_agbin_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("d")).unwrap();
    std::fs::write(base.join("hello.txt"), b"agent works").unwrap();
    std::fs::write(base.join("d/x.bin"), vec![9u8; 300]).unwrap();
    let root = base.to_string_lossy().to_string();

    let mut child = match Command::new(bin)
        .arg("--serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let w: Box<dyn Write + Send> = Box::new(child.stdin.take().unwrap());
    let r: Box<dyn Read + Send> = Box::new(child.stdout.take().unwrap());
    let inner: crate::vfs::BackendHandle = std::sync::Arc::new(crate::vfs::LocalBackend::new("/"));
    let be = AgentBackend::from_streams(r, w, inner).unwrap();
    assert!(be.version().contains('.'));

    let mut entries = be.list_dir(&root).unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries.len(), 2);

    let tree = crate::analytics::from_wire(
        be.walk_tree(&root, &|_, _| true)
            .unwrap()
            .expect("agent tree"),
    );
    assert_eq!(tree.size, 311);

    let mut buf = String::new();
    be.open_read(&format!("{}/hello.txt", root))
        .unwrap()
        .read_to_string(&mut buf)
        .unwrap();
    assert_eq!(buf, "agent works");

    {
        let mut w = be.open_write(&format!("{}/up.txt", root)).unwrap();
        w.write_all(b"streamed up").unwrap();
        w.flush().unwrap();
    }
    assert_eq!(std::fs::read(base.join("up.txt")).unwrap(), b"streamed up");
    let getdst = std::env::temp_dir().join(format!("se_got_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&getdst);
    assert_eq!(be.get_tree(&root, &getdst).unwrap(), 3);
    assert!(getdst.join("d/x.bin").exists());

    {
        let (htx, hrx) = unbounded();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert!(be.walk_hashed(&root, true, htx, &cancel).unwrap());
        let hits: Vec<crate::vfs::HashHit> = hrx.iter().collect();
        let hello = hits.iter().find(|h| h.rel == "hello.txt").unwrap();
        assert_eq!(
            hello.md5.as_deref(),
            Some(format!("{:x}", md5::compute(b"agent works")).as_str())
        );
    }

    drop(be);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&getdst);
}
