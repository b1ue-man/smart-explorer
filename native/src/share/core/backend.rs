use std::io::{self, Read, Write};

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};

use super::core::eio;
use super::protocol::{Channel, TAG_CTRL, TAG_DATA};
use super::transfer::dial_candidates;
use super::types::RemoteDevice;
use super::wire::{Ctrl, FsMeta, FsRequest, FsResponse};

pub struct PeerBackend {
    peer: RemoteDevice,
    psk: [u8; 32],
}

impl PeerBackend {
    pub(crate) fn new(peer: RemoteDevice, psk: [u8; 32]) -> Self {
        Self { peer, psk }
    }

    fn channel(&self) -> io::Result<Channel> {
        let stream = dial_candidates(&self.peer.candidates)?;
        Channel::initiator(stream, &self.psk)
    }

    fn request(&self, req: FsRequest) -> io::Result<FsResponse> {
        let mut ch = self.channel()?;
        send_req(&mut ch, req)?;
        recv_resp(&mut ch)
    }
}

impl Backend for PeerBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".to_string()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.request(FsRequest::ListDir {
            path: path.to_string(),
        })? {
            FsResponse::Entries { entries } => Ok(entries.into_iter().map(Into::into).collect()),
            _ => Err(eio("unerwartete Antwort auf list_dir")),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match self.request(FsRequest::Stat {
            path: path.to_string(),
        })? {
            FsResponse::Meta { meta } => Ok(meta.into()),
            _ => Err(eio("unerwartete Antwort auf stat")),
        }
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let mut ch = self.channel()?;
        send_req(
            &mut ch,
            FsRequest::Read {
                path: path.to_string(),
            },
        )?;
        let size = match recv_resp(&mut ch)? {
            FsResponse::Data { size } => size,
            _ => return Err(eio("unerwartete Antwort auf read")),
        };
        Ok(Box::new(PeerReader {
            ch,
            remaining: size,
            buf: Vec::new(),
            pos: 0,
        }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let mut ch = self.channel()?;
        send_req(
            &mut ch,
            FsRequest::Write {
                path: path.to_string(),
            },
        )?;
        match recv_resp(&mut ch)? {
            FsResponse::Ready => Ok(Box::new(PeerWriter {
                ch: Some(ch),
                finished: false,
            })),
            _ => Err(eio("unerwartete Antwort auf write")),
        }
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let mut r = self.open_read(src)?;
        let mut w = self.open_write(dst)?;
        let n = io::copy(&mut r, &mut w)?;
        w.flush()?;
        Ok(n)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        match self.request(FsRequest::Rename {
            src: src.to_string(),
            dst: dst.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf rename")),
        }
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        match self.request(FsRequest::RemoveFile {
            path: path.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf remove_file")),
        }
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        match self.request(FsRequest::RemoveDir {
            path: path.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf remove_dir")),
        }
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        match self.request(FsRequest::MkdirAll {
            path: path.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf mkdir_all")),
        }
    }

    fn parallelism(&self) -> usize {
        4
    }
}

struct PeerReader {
    ch: Channel,
    remaining: u64,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for PeerReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Ok(0);
        }
        while self.pos >= self.buf.len() {
            let (tag, payload) = self.ch.recv()?;
            if tag != TAG_DATA {
                return Err(eio("unerwarteter Frame beim Lesen"));
            }
            if payload.len() as u64 > self.remaining {
                return Err(eio("Peer sendet mehr Daten als angekuendigt"));
            }
            self.buf = payload;
            self.pos = 0;
            if self.buf.is_empty() && self.remaining > 0 {
                continue;
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        self.remaining = self.remaining.saturating_sub(n as u64);
        Ok(n)
    }
}

struct PeerWriter {
    ch: Option<Channel>,
    finished: bool,
}

impl PeerWriter {
    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        let ch = self
            .ch
            .as_mut()
            .ok_or_else(|| eio("Peer-Schreibkanal geschlossen"))?;
        send_req(ch, FsRequest::WriteDone)?;
        match recv_resp(ch)? {
            FsResponse::Ok => {
                self.finished = true;
                self.ch = None;
                Ok(())
            }
            _ => Err(eio("unerwartete Antwort auf Schreib-Ende")),
        }
    }
}

impl Write for PeerWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(eio("Peer-Schreibkanal ist bereits abgeschlossen"));
        }
        if let Some(ch) = self.ch.as_mut() {
            for chunk in buf.chunks(60_000) {
                ch.send(TAG_DATA, chunk)?;
            }
            Ok(buf.len())
        } else {
            Err(eio("Peer-Schreibkanal geschlossen"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for PeerWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn send_req(ch: &mut Channel, req: FsRequest) -> io::Result<()> {
    ch.send(
        TAG_CTRL,
        &serde_json::to_vec(&Ctrl::Fs { req }).map_err(eio)?,
    )
}

fn recv_resp(ch: &mut Channel) -> io::Result<FsResponse> {
    let (tag, payload) = ch.recv()?;
    if tag != TAG_CTRL {
        return Err(eio("Peer sendet keinen Steuerframe"));
    }
    match serde_json::from_slice::<Ctrl>(&payload).map_err(eio)? {
        Ctrl::FsResp {
            resp: FsResponse::Err { msg },
        } => Err(eio(msg)),
        Ctrl::FsResp { resp } => Ok(resp),
        _ => Err(eio("Peer sendet falsche Antwort")),
    }
}

impl From<FsMeta> for VfsMeta {
    fn from(m: FsMeta) -> Self {
        VfsMeta {
            name: m.name,
            is_dir: m.is_dir,
            is_symlink: m.is_symlink,
            size: m.size,
            mtime_ms: m.mtime_ms,
            btime_ms: m.btime_ms,
            hidden: m.hidden,
            system: m.system,
            id: m.id,
            content_md5: None,
        }
    }
}
