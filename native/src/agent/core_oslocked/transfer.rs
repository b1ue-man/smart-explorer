use super::backend::AgentBackend;
use super::mux::Mux;
use crate::agent_proto::{self, Frame};
use std::io::{self, Read, Write};
use std::path::Path;

impl AgentBackend {
    /// Run a single-shot mutation op that replies `Ok`/`Err`.
    pub(super) fn agent_unit_op(&self, req: Frame) -> io::Result<bool> {
        match self.mux.call(req) {
            Ok(Frame::Ok) => Ok(true),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            _ => Ok(false),
        }
    }

    /// Stream an entire remote subtree (`root`) down into local `dst`.
    pub(super) fn agent_get_tree(&self, root: &str, dst: &Path) -> io::Result<u64> {
        std::fs::create_dir_all(dst)?;
        let (id, rx) = self.mux.register();
        let r = (|| {
            self.mux.send(id, Frame::GetTree(root.to_string()))?;
            let mut cur: Option<std::fs::File> = None;
            let mut files = 0u64;
            loop {
                match rx.recv() {
                    Ok(Frame::TreeEntry { rel, is_dir, .. }) => {
                        cur = None;
                        let p = dst.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                        if is_dir {
                            std::fs::create_dir_all(&p)?;
                        } else {
                            if let Some(parent) = p.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            cur = Some(std::fs::File::create(&p)?);
                            files += 1;
                        }
                    }
                    Ok(Frame::Data(d)) => {
                        if let Some(f) = cur.as_mut() {
                            f.write_all(&d)?;
                        }
                    }
                    Ok(Frame::End) => break,
                    Ok(Frame::Err(e)) => return Err(io::Error::other(e)),
                    Ok(_) => {}
                    Err(_) => {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent get-tree closed"))
                    }
                }
            }
            Ok(files)
        })();
        self.mux.unregister(id);
        r
    }

    /// Stream an entire local subtree (`src`) up into remote `root`.
    pub(super) fn agent_put_tree(&self, src: &Path, root: &str) -> io::Result<u64> {
        let (id, rx) = self.mux.register();
        let r = (|| {
            self.mux.send(id, Frame::PutTree(root.to_string()))?;
            let mut files = 0u64;
            send_subtree(&self.mux, id, src, src, &mut files)?;
            self.mux.send(id, Frame::End)?;
            match rx.recv() {
                Ok(Frame::Ok) => Ok(files),
                Ok(Frame::Err(e)) => Err(io::Error::other(e)),
                _ => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent put-tree closed")),
            }
        })();
        self.mux.unregister(id);
        r
    }
}

/// Depth-first walk of a local subtree, emitting `TreeEntry` and `Data` chunks.
fn send_subtree(mux: &Mux, id: u64, base: &Path, dir: &Path, files: &mut u64) -> io::Result<()> {
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        let p = ent.path();
        let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        if ft.is_dir() {
            mux.send(id, Frame::TreeEntry { rel, is_dir: true, size: 0, mtime_ms: 0 })?;
            send_subtree(mux, id, base, &p, files)?;
        } else if ft.is_file() {
            let md = ent.metadata().ok();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            mux.send(id, Frame::TreeEntry { rel, is_dir: false, size, mtime_ms: 0 })?;
            let mut f = std::fs::File::open(&p)?;
            let mut buf = vec![0u8; agent_proto::CHUNK];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                mux.send(id, Frame::Data(buf[..n].to_vec()))?;
            }
            *files += 1;
        }
    }
    Ok(())
}
