use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

use super::fs::{is_pseudo_dir, systemtime_ms};
use super::session::{emit, Sink};
use super::{Frame, CHUNK};

/// Read a file `[offset, offset+len)` (len 0 = to EOF) -> `Data`* then `End`.
pub(crate) fn handle_read(
    sink: &Sink,
    id: u64,
    path: &str,
    offset: u64,
    len: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut f = std::fs::File::open(path)?;
    if offset > 0 {
        f.seek(SeekFrom::Start(offset))?;
    }
    let mut remaining = if len == 0 { u64::MAX } else { len };
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let want = remaining.min(buf.len() as u64) as usize;
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
        remaining -= n as u64;
    }
    emit(sink, id, &Frame::End)
}

/// Receive a byte stream (`Data`* `End`) into `path` via a temp + atomic rename.
pub(crate) fn handle_write(
    sink: &Sink,
    id: u64,
    path: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let tmp = format!("{path}.se-agent-{id:x}.part");
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(&tmp)?;
    emit(sink, id, &Frame::Progress { done: 0, total: 0 })?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Ok(());
        }
        match inbound.recv() {
            Ok(Frame::Data(d)) => f.write_all(&d)?,
            Ok(Frame::End) => break,
            Ok(_) => {}
            Err(_) => {
                drop(f);
                let _ = std::fs::remove_file(&tmp);
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "upload aborted",
                ));
            }
        }
    }
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    emit(sink, id, &Frame::Ok)
}

pub(crate) fn remove_path(path: &str, recursive: bool) -> io::Result<()> {
    let md = std::fs::symlink_metadata(path)?;
    if md.is_dir() {
        if recursive {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_dir(path)
        }
    } else {
        std::fs::remove_file(path)
    }
}

/// Stream an entire subtree down.
pub(crate) fn handle_get_tree(
    sink: &Sink,
    id: u64,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    fn walk(sink: &Sink, id: u64, base: &Path, dir: &Path, cancel: &AtomicBool) -> io::Result<()> {
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };
        for ent in rd.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let p = ent.path();
            let rel = p
                .strip_prefix(base)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            if ft.is_dir() {
                if is_pseudo_dir(&p.to_string_lossy()) {
                    continue;
                }
                emit(
                    sink,
                    id,
                    &Frame::TreeEntry {
                        rel,
                        is_dir: true,
                        size: 0,
                        mtime_ms: 0,
                    },
                )?;
                walk(sink, id, base, &p, cancel)?;
            } else if ft.is_file() {
                let md = ent.metadata().ok();
                let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
                let mtime = md
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .map(systemtime_ms)
                    .unwrap_or(0);
                emit(
                    sink,
                    id,
                    &Frame::TreeEntry {
                        rel,
                        is_dir: false,
                        size,
                        mtime_ms: mtime,
                    },
                )?;
                let mut f = std::fs::File::open(&p)?;
                let mut buf = vec![0u8; CHUNK];
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let n = f.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
                }
            }
        }
        Ok(())
    }
    let base = Path::new(root);
    walk(sink, id, base, base, cancel)?;
    emit(sink, id, &Frame::End)
}

/// Receive an entire subtree under `root`.
pub(crate) fn handle_put_tree(
    sink: &Sink,
    id: u64,
    root: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    let base = Path::new(root);
    let mut cur: Option<std::fs::File> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match inbound.recv() {
            Ok(Frame::TreeEntry { rel, is_dir, .. }) => {
                cur = None;
                let dst = base.join(&rel);
                if is_dir {
                    std::fs::create_dir_all(&dst)?;
                } else {
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    cur = Some(std::fs::File::create(&dst)?);
                }
            }
            Ok(Frame::Data(d)) => {
                if let Some(f) = cur.as_mut() {
                    f.write_all(&d)?;
                }
            }
            Ok(Frame::End) => break,
            Ok(_) => {}
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "put-tree aborted",
                ))
            }
        }
    }
    emit(sink, id, &Frame::Ok)
}
