use super::*;
use crate::vfs::Backend;
use crossbeam_channel::unbounded;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

#[test]
fn artifact_selection_and_quoting() {
    let a = artifact_for("Linux x86_64").expect("x86_64 bundled");
    assert!(a.bytes.len() > 1000 && a.bytes.starts_with(b"\x7fELF"));
    assert!(artifact_for("Linux aarch64").is_some());
    assert!(artifact_for("Darwin arm64").is_none());
    assert!(artifact_for("garbage").is_none());
    assert_eq!(super::deploy::sha256_hex(a.bytes).len(), 64);
    assert_eq!(super::deploy::sh_quote("/home/u/dir"), "'/home/u/dir'");
    assert_eq!(super::deploy::sh_quote("a'b; rm -rf /"), r#"'a'\''b; rm -rf /'"#);
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
    assert_eq!(entries.iter().find(|e| e.name == "a.txt").unwrap().size, 100);
    assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

    assert!(be.supports_walk_tree());
    let tree = crate::analytics::from_wire(be.walk_tree(&root, &|_, _| true).unwrap());
    assert_eq!(tree.size, 500);
    assert_eq!(tree.children.iter().find(|c| &*c.name == "sub").unwrap().size, 400);

    let m = be.stat(&format!("{}/a.txt", root)).unwrap();
    assert_eq!(m.size, 100);
    assert!(!m.is_dir);

    let mut buf = Vec::new();
    be.open_read(&format!("{}/a.txt", root)).unwrap().read_to_end(&mut buf).unwrap();
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
        assert!(be.search(&root, &spec, stx, &cancel));
        let hits: Vec<String> = srx.iter().map(|h| h.rel).collect();
        assert_eq!(hits, vec!["sub/b.bin".to_string()]);
    }

    {
        let (htx, hrx) = unbounded();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert!(be.walk_hashed(&root, true, htx, &cancel));
        let hits: Vec<crate::vfs::HashHit> = hrx.iter().collect();
        let files: Vec<&crate::vfs::HashHit> = hits.iter().filter(|h| !h.is_dir).collect();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|h| h.md5.as_ref().map_or(false, |m| m.len() == 32)));
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
    assert_eq!(std::fs::read(base.join("written.dat")).unwrap(), b"hello agent write");

    be.mkdir_all(&format!("{}/newdir/inner", root)).unwrap();
    assert!(base.join("newdir/inner").is_dir());
    be.copy_file(&format!("{}/a.txt", root), &format!("{}/newdir/copy.txt", root)).unwrap();
    assert_eq!(std::fs::read(base.join("newdir/copy.txt")).unwrap().len(), 100);
    be.rename(&format!("{}/newdir/copy.txt", root), &format!("{}/newdir/moved.txt", root)).unwrap();
    assert!(!base.join("newdir/copy.txt").exists() && base.join("newdir/moved.txt").exists());
    be.remove_file(&format!("{}/newdir/moved.txt", root)).unwrap();
    assert!(!base.join("newdir/moved.txt").exists());
    assert!(be.remove_dir(&format!("{}/newdir", root)).is_err());

    assert!(be.supports_bulk_tree());
    let upsrc = base.join("upsrc");
    std::fs::create_dir_all(upsrc.join("sub")).unwrap();
    std::fs::write(upsrc.join("f1.txt"), b"one").unwrap();
    std::fs::write(upsrc.join("sub/f2.txt"), b"two longer").unwrap();
    let remote_dst = format!("{}/uploaded", root);
    assert_eq!(be.put_tree(&upsrc, &remote_dst).unwrap(), 2);
    assert_eq!(std::fs::read(base.join("uploaded/f1.txt")).unwrap(), b"one");
    assert_eq!(std::fs::read(base.join("uploaded/sub/f2.txt")).unwrap(), b"two longer");
    let getdst = base.join("downloaded");
    assert_eq!(be.get_tree(&remote_dst, &getdst).unwrap(), 2);
    assert_eq!(std::fs::read(getdst.join("f1.txt")).unwrap(), b"one");
    assert_eq!(std::fs::read(getdst.join("sub/f2.txt")).unwrap(), b"two longer");

    drop(be);
    let _ = shut.shutdown(std::net::Shutdown::Both);
    let _ = server.join();
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn real_agent_binary_child_process() {
    use std::process::{Command, Stdio};
    let bin = concat!(env!("CARGO_MANIFEST_DIR"), "/agent-bin/se-agent-x86_64-linux-musl");
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

    let tree = crate::analytics::from_wire(be.walk_tree(&root, &|_, _| true).unwrap());
    assert_eq!(tree.size, 311);

    let mut buf = String::new();
    be.open_read(&format!("{}/hello.txt", root)).unwrap().read_to_string(&mut buf).unwrap();
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
        assert!(be.walk_hashed(&root, true, htx, &cancel));
        let hits: Vec<crate::vfs::HashHit> = hrx.iter().collect();
        let hello = hits.iter().find(|h| h.rel == "hello.txt").unwrap();
        assert_eq!(hello.md5.as_deref(), Some(format!("{:x}", md5::compute(b"agent works")).as_str()));
    }

    drop(be);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&getdst);
}
