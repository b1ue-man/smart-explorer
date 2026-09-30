//! Streamed single-file reads and writes of the background service's backend
//! server (moved out of `backend_server.rs`; a read now starts at its offset
//! directly where the backend can).
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use crate::agent_proto::{Frame, Inbound, CHUNK};
use crate::vfs::BackendHandle;

use super::backend_server::{emit, Sink};

pub(super) fn handle_read_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    path: &str,
    offset: u64,
    len: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) { return Ok(()); }
    let mut r = match offset {
        0 => backend.open_read(path)?,
        _ => match backend.open_read_at(path, None, offset)? {
            Some(reader) => reader,
            None => {
                // The backend cannot start mid-file: read and drop the prefix.
                let mut reader = backend.open_read(path)?;
                let mut remaining = offset;
                let mut discard = vec![0u8; CHUNK];
                while remaining > 0 {
                    if cancel.load(Ordering::Relaxed) { return Ok(()); }
                    let want = remaining.min(discard.len() as u64) as usize;
                    let read = match reader.read(&mut discard[..want]) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        result => result?,
                    };
                    if read == 0 { break; }
                    remaining -= read as u64;
                }
                reader
            }
        },
    };
    let mut remaining = if len == 0 { u64::MAX } else { len };
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let want = remaining.min(buf.len() as u64) as usize;
        let n = r.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
        remaining -= n as u64;
    }
    emit(sink, id, &Frame::End)
}

#[derive(Clone, Copy)]
pub(super) enum WriteMode {
    Replace,
    Create,
}

pub(super) fn handle_write_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    path: &str,
    inbound: &dyn Inbound,
    cancel: &AtomicBool,
    mode: WriteMode,
) -> io::Result<()> {
    let (staged, mut writer, replace_after_upload) = match mode {
        WriteMode::Replace => {
            let staged = crate::vfs::unique_staging_path(&**backend, path, "daemon")?;
            let writer = backend.open_write_new(&staged)?;
            (staged, writer, true)
        }
        WriteMode::Create => {
            let writer = backend.open_write_new(path)?;
            (path.to_string(), writer, false)
        }
    };
    if let Err(error) = emit(sink, id, &Frame::Progress { done: 0, total: 0 }) {
        drop(writer);
        // The exclusive open transferred ownership at the backend boundary,
        // but this generic layer has no stable item identity. Retain the entry:
        // a concurrent actor may already have moved it and reused its spelling.
        return Err(error);
    }
    let transfer = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "upload canceled",
            ));
        }
        match inbound.recv_timeout(Duration::from_millis(100)) {
            Ok(Frame::Data(data)) => {
                if let Err(error) = writer.write_all(&data) {
                    break Err(error);
                }
            }
            Ok(Frame::End) => break writer.flush(),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "daemon backend upload aborted",
                ));
            }
        }
    };
    drop(writer);
    // A nested writer may have committed before its final acknowledgement
    // was lost. Path-based cleanup could delete a replacement entry.
    transfer?;
    let promotion = if replace_after_upload {
        crate::vfs::promote_staged_replace(&**backend, &staged, path)
    } else {
        Ok(())
    };
    // Promotion responses are ambiguous across reconnects. Preserve the
    // staging name as recovery evidence instead of deleting by spelling.
    promotion?;
    emit(sink, id, &Frame::Ok)
}
