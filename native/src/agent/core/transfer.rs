use super::backend::AgentBackend;
use super::mux::Mux;
use crate::agent_proto::{self, BufferedTreeReceiver, Frame, LocalTreeEntry};
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::AtomicBool;

impl AgentBackend {
    /// Run a single-shot mutation op that replies `Ok`/`Err`. Once protocol-v7
    /// has handshaken, a missing or malformed reply is an ambiguous remote
    /// completion and must never be retried through the wrapped backend.
    pub(super) fn agent_unit_op(&self, req: Frame) -> io::Result<()> {
        let (mux, reply) = self.connection.mutation_call(req)?;
        match reply {
            Frame::Ok => Ok(()),
            Frame::Err(e) => Err(io::Error::other(e)),
            other => {
                self.connection.invalidate(&mux);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected agent mutation reply: {other:?}"),
                ))
            }
        }
    }

    /// Stream an entire remote subtree (`root`) down into local `dst`.
    pub(super) fn agent_get_tree(&self, root: &str, dst: &Path) -> io::Result<u64> {
        agent_proto::validate_destination_root(dst)?;
        let mux = self.connection.mux()?;
        let (id, rx) = mux.register();
        let r = (|| {
            let mut receiver = BufferedTreeReceiver::create("download", id)?;
            mux.send(id, Frame::GetTree(root.to_string()))?;
            loop {
                match rx.recv() {
                    Ok(frame @ Frame::TreeEntry { .. }) | Ok(frame @ Frame::Data(_)) => {
                        receiver.accept(frame)?;
                    }
                    Ok(Frame::End) => {
                        receiver.accept(Frame::End)?;
                        break;
                    }
                    Ok(Frame::Err(e)) => return Err(io::Error::other(e)),
                    Ok(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected frame in agent get-tree stream",
                        ));
                    }
                    Err(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent get-tree closed",
                        ))
                    }
                }
            }
            receiver.finish()?.publish_local(dst, "download", id)
        })();
        if r.is_err() {
            let _ = mux.send(id, Frame::Cancel);
        }
        mux.unregister(id);
        r
    }

    /// Stream an entire local subtree (`src`) up into remote `root`.
    pub(super) fn agent_put_tree(&self, src: &Path, root: &str) -> io::Result<u64> {
        let entries = agent_proto::collect_local_tree(src, &AtomicBool::new(false))?;
        let mux = self.connection.mutation_mux()?;
        let (id, rx) = mux.register();
        let r = (|| {
            mux.send(id, Frame::PutTree(root.to_string()))?;
            let mut files = 0u64;
            if let Err(error) = send_tree_manifest(&mux, id, src, &entries, &mut files) {
                let _ = mux.send(id, Frame::Cancel);
                let _ = mux.send(id, Frame::End);
                return Err(error);
            }
            mux.send(id, Frame::End)?;
            match rx.recv() {
                Ok(Frame::Ok) => Ok(files),
                Ok(Frame::Err(e)) => Err(io::Error::other(e)),
                Ok(other) => {
                    self.connection.invalidate(&mux);
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected agent put-tree reply: {other:?}"),
                    ))
                }
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "agent put-tree closed",
                )),
            }
        })();
        if r.is_err() && mux.is_closed() {
            self.connection.invalidate(&mux);
        }
        mux.unregister(id);
        r
    }
}

fn send_tree_manifest(
    mux: &Mux,
    id: u64,
    src: &Path,
    entries: &[LocalTreeEntry],
    files: &mut u64,
) -> io::Result<()> {
    let mut buffer = vec![0u8; agent_proto::CHUNK];
    for entry in entries {
        let mut file = if entry.is_dir {
            None
        } else {
            Some(agent_proto::open_local_tree_file(src, entry)?)
        };
        mux.send(
            id,
            Frame::TreeEntry {
                rel: entry.relative.as_str().to_string(),
                is_dir: entry.is_dir,
                size: entry.size,
                mtime_ms: entry.mtime_ms,
            },
        )?;
        let Some(mut file) = file.take() else {
            continue;
        };
        let mut sent = 0u64;
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            mux.send(id, Frame::Data(buffer[..read].to_vec()))?;
            sent = sent
                .checked_add(read as u64)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file size overflow"))?;
        }
        agent_proto::finish_local_tree_file(src, entry, &file, sent)?;
        *files = (*files)
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file count overflow"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::AgentBackend;
    use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    #[test]
    fn disconnected_get_tree_preserves_existing_local_destination() {
        let destination_root = std::env::temp_dir().join(format!(
            "se_agent_get_disconnect_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&destination_root);
        std::fs::create_dir_all(&destination_root).unwrap();
        let destination = destination_root.join("file.txt");
        std::fs::write(&destination, b"old").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = socket.try_clone().unwrap();
            let (hello_id, _) = read_frame(&mut reader).unwrap().unwrap();
            write_frame(
                &mut socket,
                hello_id,
                &Frame::HelloOk {
                    proto: PROTO_VERSION,
                    version: "test".into(),
                },
            )
            .unwrap();
            let (request_id, request) = read_frame(&mut reader).unwrap().unwrap();
            assert!(matches!(request, Frame::GetTree(_)));
            write_frame(
                &mut socket,
                request_id,
                &Frame::TreeEntry {
                    rel: "file.txt".into(),
                    is_dir: false,
                    size: 3,
                    mtime_ms: 0,
                },
            )
            .unwrap();
            write_frame(&mut socket, request_id, &Frame::Data(b"new".to_vec())).unwrap();
        });

        let client = TcpStream::connect(address).unwrap();
        let backend = AgentBackend::from_streams(
            Box::new(client.try_clone().unwrap()) as Box<dyn Read + Send>,
            Box::new(client) as Box<dyn Write + Send>,
            Arc::new(crate::vfs::LocalBackend::new("/")),
        )
        .unwrap();
        assert!(backend
            .agent_get_tree("/remote", &destination_root)
            .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        assert!(!std::fs::read_dir(&destination_root)
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains(".se-agent-download-")));

        drop(backend);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(destination_root);
    }
}
