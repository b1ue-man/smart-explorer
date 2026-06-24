use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use iroh::endpoint::{presets, Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, TransportAddr};
use tokio::io::AsyncWriteExt;

use super::core::{eio, hmac_proof, random_token, verify_hmac};
use super::fs::{self, ShareExportConfig};
use super::identity::ShareIdentity;
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::types::{
    DirectGrantState, PeerEndpoint, ShareAuthState, ShareEvent, ShareScope, ShareStatus,
};
use super::wire::{Ctrl, FsMeta, FsRequest, FsResponse, PeerHello};

const ALPN: &[u8] = b"smart-explorer/share-fs/3";
const TAG_CTRL: u8 = 0;
const TAG_DATA: u8 = 1;
const MAX_FRAME: usize = 16 * 1024 * 1024;

pub struct PeerBackend {
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    node: Arc<ShareIrohNode>,
}

impl PeerBackend {
    pub(crate) fn new(
        endpoint: PeerEndpoint,
        identity: ShareIdentity,
        node: Arc<ShareIrohNode>,
    ) -> Self {
        Self {
            endpoint,
            identity,
            node,
        }
    }

    pub(crate) fn probe_root(&self) -> io::Result<Vec<VfsMeta>> {
        self.list_dir("/")
    }

    pub(crate) fn transport_status(&self) -> ShareStatus {
        self.node
            .session_transport(&self.endpoint)
            .map(|t| match t {
                "relay" => ShareStatus::ConnectedRelay,
                "direct" => ShareStatus::ConnectedDirect,
                _ => ShareStatus::Connected,
            })
            .unwrap_or(ShareStatus::Connected)
    }

    fn request(&self, req: FsRequest) -> io::Result<FsResponse> {
        let op = fs_request_label(&req);
        let started = Instant::now();
        let (mut send, mut recv) = self.node.open_stream(&self.endpoint, &self.identity)?;
        let resp = self.node.block_on(async {
            send_ctrl(&mut send, &Ctrl::Fs { req }).await?;
            recv_resp(&mut recv).await
        })?;
        let _ = self.node.ev.send(ShareEvent::Status(format!(
            "Share-Op {op}: {} ms, {}",
            started.elapsed().as_millis(),
            fs_response_summary(&resp)
        )));
        Ok(resp)
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
        let (mut send, mut recv) = self.node.open_stream(&self.endpoint, &self.identity)?;
        let size = self.node.block_on(async {
            send_ctrl(
                &mut send,
                &Ctrl::Fs {
                    req: FsRequest::Read {
                        path: path.to_string(),
                    },
                },
            )
            .await?;
            match recv_resp(&mut recv).await? {
                FsResponse::Data { size } => Ok(size),
                _ => Err(eio("unerwartete Antwort auf read")),
            }
        })?;
        Ok(Box::new(PeerReader {
            node: self.node.clone(),
            recv,
            remaining: size,
            buf: Vec::new(),
            pos: 0,
        }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let (mut send, mut recv) = self.node.open_stream(&self.endpoint, &self.identity)?;
        self.node.block_on(async {
            send_ctrl(
                &mut send,
                &Ctrl::Fs {
                    req: FsRequest::Write {
                        path: path.to_string(),
                    },
                },
            )
            .await?;
            match recv_resp(&mut recv).await? {
                FsResponse::Ready => Ok(()),
                _ => Err(eio("unerwartete Antwort auf write")),
            }
        })?;
        Ok(Box::new(PeerWriter {
            node: self.node.clone(),
            send: Some(send),
            recv: Some(recv),
            finished: false,
        }))
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        match self.request(FsRequest::CopyFile {
            src: src.to_string(),
            dst: dst.to_string(),
        })? {
            FsResponse::Data { size } => Ok(size),
            FsResponse::Ok => Ok(self.stat(dst).map(|m| m.size).unwrap_or(0)),
            _ => Err(eio("unerwartete Antwort auf copy_file")),
        }
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
        8
    }

    fn rename_overwrites(&self) -> bool {
        true
    }
}

pub(crate) struct ShareIrohNode {
    rt: Arc<tokio::runtime::Runtime>,
    endpoint: Endpoint,
    auth: Arc<Mutex<ShareAuthState>>,
    ev: crossbeam_channel::Sender<ShareEvent>,
    sessions: Mutex<HashMap<String, Connection>>,
    accept_errors: Mutex<HashMap<String, (Instant, u32)>>,
    relay_url: String,
}

impl ShareIrohNode {
    pub(crate) fn start(
        server: &str,
        identity: &ShareIdentity,
        auth: Arc<Mutex<ShareAuthState>>,
        ev: crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<Arc<Self>> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("share-iroh")
                .build()
                .map_err(eio)?,
        );
        let relay_url = relay_url_from_signal(server);
        let relay_mode = relay_url
            .parse::<RelayUrl>()
            .ok()
            .map(|url| RelayMode::Custom(RelayMap::from(url)))
            .unwrap_or(RelayMode::Disabled);
        let endpoint = rt.block_on(async {
            Endpoint::builder(presets::Minimal)
                .secret_key(identity.iroh_secret.clone())
                .alpns(vec![ALPN.to_vec()])
                .relay_mode(relay_mode)
                .bind()
                .await
                .map_err(eio)
        })?;
        let node = Arc::new(Self {
            rt,
            endpoint,
            auth,
            ev,
            sessions: Mutex::new(HashMap::new()),
            accept_errors: Mutex::new(HashMap::new()),
            relay_url,
        });
        node.spawn_accept_loop();
        Ok(node)
    }

    pub(crate) fn relay_url(&self) -> &str {
        &self.relay_url
    }

    pub(crate) fn candidates(&self) -> Vec<String> {
        self.endpoint
            .addr()
            .ip_addrs()
            .map(|addr| addr.to_string())
            .collect()
    }

    fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }

    fn open_stream(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
    ) -> io::Result<(SendStream, RecvStream)> {
        let key = session_key(endpoint);
        let conn = if let Some(conn) = self.sessions.lock().ok().and_then(|s| s.get(&key).cloned())
        {
            conn
        } else {
            let conn = self.connect_session(endpoint, identity)?;
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.insert(key.clone(), conn.clone());
            }
            conn
        };
        match self.block_on(async { conn.open_bi().await.map_err(eio) }) {
            Ok(streams) => Ok(streams),
            Err(_) => {
                if let Ok(mut sessions) = self.sessions.lock() {
                    sessions.remove(&key);
                }
                let conn = self.connect_session(endpoint, identity)?;
                if let Ok(mut sessions) = self.sessions.lock() {
                    sessions.insert(key, conn.clone());
                }
                self.block_on(async { conn.open_bi().await.map_err(eio) })
            }
        }
    }

    fn session_transport(&self, endpoint: &PeerEndpoint) -> Option<&'static str> {
        let key = session_key(endpoint);
        let conn = self.sessions.lock().ok()?.get(&key).cloned()?;
        Some(transport_label(&conn))
    }

    fn connect_session(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
    ) -> io::Result<Connection> {
        if let Some(expected) = endpoint.expected_node_id.as_deref() {
            if !expected.trim().is_empty() && expected != endpoint.presence.node_id {
                return Err(eio("Iroh NodeId passt nicht zur gepinnten Identitaet"));
            }
        }
        let addr = endpoint_addr(&endpoint.presence)?;
        let (kind, relation_id) = relation_kind_id(endpoint);
        let remote_device = endpoint.presence.device_id.clone();
        let remote_node = endpoint.presence.node_id.clone();
        let nonce = random_token(12);
        let payload = session_payload(
            kind,
            &relation_id,
            &identity.device_id,
            &remote_device,
            &identity.node_id,
            &remote_node,
            &nonce,
        );
        let proof = hmac_proof(&endpoint.relation_secret, &payload);
        let hello = PeerHello {
            protocol_version: 3,
            relation_kind: kind.to_string(),
            relation_id,
            device_id: identity.device_id.clone(),
            public_key: identity.public_key.clone(),
            node_id: identity.node_id.clone(),
            session_nonce: nonce,
            session_proof: proof,
            requested_capabilities: vec!["fs".to_string()],
        };
        let started = Instant::now();
        self.block_on(async {
            let conn = self.endpoint.connect(addr, ALPN).await.map_err(eio)?;
            let (mut send, mut recv) = conn.open_bi().await.map_err(eio)?;
            send_ctrl(&mut send, &Ctrl::PeerHello { hello }).await?;
            match recv_ctrl(&mut recv).await? {
                Ctrl::PeerHelloOk => {
                    let transport = transport_label(&conn);
                    let _ = self.ev.send(ShareEvent::Status(format!(
                        "Iroh-Session authentifiziert: {} via {} in {} ms",
                        remote_device,
                        transport,
                        started.elapsed().as_millis()
                    )));
                    Ok(conn)
                }
                Ctrl::FsResp {
                    resp: FsResponse::Err { msg },
                } => Err(eio(msg)),
                _ => Err(eio("Peer akzeptiert die Iroh-Session nicht")),
            }
        })
    }

    fn spawn_accept_loop(self: &Arc<Self>) {
        let node = self.clone();
        self.rt.spawn(async move {
            while let Some(incoming) = node.endpoint.accept().await {
                let node = node.clone();
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(conn) => {
                            if let Err(e) = node.clone().handle_connection(conn).await {
                                let _ = node.ev.send(ShareEvent::Error(format!("Iroh-Peer: {e}")));
                            }
                        }
                        Err(e) => {
                            node.emit_accept_error(e.to_string());
                        }
                    }
                });
            }
        });
    }

    fn emit_accept_error(&self, msg: String) {
        let mut send = Some(format!("Iroh-Accept: {msg}"));
        if let Ok(mut seen) = self.accept_errors.lock() {
            let now = Instant::now();
            let entry = seen.entry(msg.clone()).or_insert((now, 0));
            entry.1 = entry.1.saturating_add(1);
            if entry.1 > 1 && entry.0.elapsed() < Duration::from_secs(30) {
                send = None;
            } else if entry.1 > 1 {
                send = Some(format!(
                    "Iroh-Accept: {msg} ({} Wiederholungen in 30s)",
                    entry.1
                ));
                *entry = (now, 1);
            } else {
                entry.0 = now;
            }
        }
        if let Some(msg) = send {
            let _ = self.ev.send(ShareEvent::Error(msg));
        }
    }

    async fn handle_connection(self: Arc<Self>, conn: Connection) -> io::Result<()> {
        let remote_node = conn.remote_id().to_string();
        let (mut send, mut recv) = tokio::time::timeout(Duration::from_secs(20), conn.accept_bi())
            .await
            .map_err(|_| eio("Session-Handshake Timeout"))?
            .map_err(eio)?;
        let hello = match recv_ctrl(&mut recv).await? {
            Ctrl::PeerHello { hello } => hello,
            _ => return Err(eio("Session-Hello fehlt")),
        };
        if hello.protocol_version != 3 {
            send_ctrl(
                &mut send,
                &Ctrl::FsResp {
                    resp: FsResponse::Err {
                        msg: "Inkompatibles Share-Protokoll".into(),
                    },
                },
            )
            .await?;
            return Err(eio("Inkompatibles Share-Protokoll"));
        }
        let exports = match resolve_incoming_session(&hello, &remote_node, &self.auth) {
            Ok(exports) => exports,
            Err(e) => {
                send_ctrl(
                    &mut send,
                    &Ctrl::FsResp {
                        resp: FsResponse::Err { msg: e.to_string() },
                    },
                )
                .await?;
                return Err(e);
            }
        };
        send_ctrl(&mut send, &Ctrl::PeerHelloOk).await?;
        let _ = self.ev.send(ShareEvent::Status(format!(
            "Iroh-Session akzeptiert: {} ({})",
            hello.device_id, remote_node
        )));
        let exports = Arc::new(Mutex::new(exports));
        loop {
            let (send, recv) = match conn.accept_bi().await {
                Ok(streams) => streams,
                Err(e) => return Err(eio(e)),
            };
            let exports = exports.clone();
            let ev = self.ev.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_fs_stream(send, recv, exports).await {
                    let _ = ev.send(ShareEvent::Error(format!("Iroh-FS: {e}")));
                }
            });
        }
    }
}

struct PeerReader {
    node: Arc<ShareIrohNode>,
    recv: RecvStream,
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
            let (tag, payload) = self
                .node
                .block_on(async { recv_tagged(&mut self.recv).await })?;
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
    node: Arc<ShareIrohNode>,
    send: Option<SendStream>,
    recv: Option<RecvStream>,
    finished: bool,
}

impl PeerWriter {
    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        let Some(mut send) = self.send.take() else {
            return Err(eio("Peer-Schreibkanal geschlossen"));
        };
        let Some(mut recv) = self.recv.take() else {
            return Err(eio("Peer-Schreibantwort geschlossen"));
        };
        self.node.block_on(async {
            send_ctrl(
                &mut send,
                &Ctrl::Fs {
                    req: FsRequest::WriteDone,
                },
            )
            .await?;
            match recv_resp(&mut recv).await? {
                FsResponse::Ok => {
                    send.finish().map_err(eio)?;
                    Ok(())
                }
                _ => Err(eio("unerwartete Antwort auf Schreib-Ende")),
            }
        })?;
        self.finished = true;
        Ok(())
    }
}

impl Write for PeerWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(eio("Peer-Schreibkanal ist bereits abgeschlossen"));
        }
        let Some(send) = self.send.as_mut() else {
            return Err(eio("Peer-Schreibkanal geschlossen"));
        };
        self.node.block_on(async {
            for chunk in buf.chunks(fs::CHUNK) {
                send_tagged(send, TAG_DATA, chunk).await?;
            }
            Ok::<(), io::Error>(())
        })?;
        Ok(buf.len())
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

async fn handle_fs_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    exports: Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let ctrl = recv_ctrl(&mut recv).await?;
    let Ctrl::Fs { req } = ctrl else {
        return Err(eio("Dateioperation erwartet"));
    };
    match req {
        FsRequest::ListDir { path } => match fs::list_dir(&path, &exports) {
            Ok(entries) => reply(&mut send, FsResponse::Entries { entries }).await,
            Err(e) => reply_err(&mut send, e).await,
        },
        FsRequest::Stat { path } => match fs::stat(&path, &exports) {
            Ok(meta) => reply(&mut send, FsResponse::Meta { meta }).await,
            Err(e) => reply_err(&mut send, e).await,
        },
        FsRequest::Read { path } => read_file(&mut send, &path, &exports).await,
        FsRequest::Write { path } => write_file(&mut send, &mut recv, &path, &exports).await,
        FsRequest::MkdirAll { path } => {
            simple(&mut send, &path, &exports, |t| t.backend.mkdir_all(&t.path)).await
        }
        FsRequest::Rename { src, dst } => {
            match (fs::resolve(&src, &exports), fs::resolve(&dst, &exports)) {
                (Ok(a), Ok(b)) if a.mount_key == b.mount_key => {
                    match a.backend.rename(&a.path, &b.path) {
                        Ok(()) => reply(&mut send, FsResponse::Ok).await,
                        Err(e) => reply_err(&mut send, e).await,
                    }
                }
                (Ok(_), Ok(_)) => {
                    reply_err(
                        &mut send,
                        eio("Quelle und Ziel liegen nicht auf derselben Freigabe"),
                    )
                    .await
                }
                (Err(e), _) | (_, Err(e)) => reply_err(&mut send, e).await,
            }
        }
        FsRequest::CopyFile { src, dst } => {
            match (fs::resolve(&src, &exports), fs::resolve(&dst, &exports)) {
                (Ok(a), Ok(b)) if a.mount_key == b.mount_key => {
                    match a.backend.copy_file(&a.path, &b.path) {
                        Ok(size) => reply(&mut send, FsResponse::Data { size }).await,
                        Err(e) => reply_err(&mut send, e).await,
                    }
                }
                (Ok(_), Ok(_)) => {
                    reply_err(
                        &mut send,
                        eio("Quelle und Ziel liegen nicht auf derselben Freigabe"),
                    )
                    .await
                }
                (Err(e), _) | (_, Err(e)) => reply_err(&mut send, e).await,
            }
        }
        FsRequest::RemoveFile { path } => {
            simple(&mut send, &path, &exports, |t| {
                t.backend.remove_file(&t.path)
            })
            .await
        }
        FsRequest::RemoveDir { path } => {
            simple(&mut send, &path, &exports, |t| {
                fs::remove_dir_recursive(&*t.backend, &t.path)
            })
            .await
        }
        FsRequest::WriteDone => reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await,
    }
}

async fn simple<F>(
    send: &mut SendStream,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
    f: F,
) -> io::Result<()>
where
    F: FnOnce(fs::ResolvedTarget) -> io::Result<()>,
{
    match fs::resolve(path, exports).and_then(f) {
        Ok(()) => reply(send, FsResponse::Ok).await,
        Err(e) => reply_err(send, e).await,
    }
}

async fn read_file(
    send: &mut SendStream,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let t = match fs::resolve(path, exports) {
        Ok(t) => t,
        Err(e) => return reply_err(send, e).await,
    };
    let size = match t.backend.stat(&t.path) {
        Ok(m) if !m.is_dir => m.size,
        Ok(_) => return reply_err(send, eio("Ordner kann nicht als Datei gelesen werden")).await,
        Err(e) => return reply_err(send, e).await,
    };
    let mut r = match t.backend.open_read(&t.path) {
        Ok(r) => r,
        Err(e) => return reply_err(send, e).await,
    };
    reply(send, FsResponse::Data { size }).await?;
    let mut buf = vec![0u8; fs::CHUNK];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        send_tagged(send, TAG_DATA, &buf[..n]).await?;
    }
    Ok(())
}

async fn write_file(
    send: &mut SendStream,
    recv: &mut RecvStream,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let t = match fs::resolve(path, exports) {
        Ok(t) => t,
        Err(e) => return reply_err(send, e).await,
    };
    let tmp_path = format!("{}.se-part-{}", t.path, random_token(8));
    let mut w = match t.backend.open_write(&tmp_path) {
        Ok(w) => Some(w),
        Err(e) => return reply_err(send, e).await,
    };
    reply(send, FsResponse::Ready).await?;
    let result = loop {
        let (tag, payload) = match recv_tagged(recv).await {
            Ok(frame) => frame,
            Err(e) => break Err(e),
        };
        if tag == TAG_DATA {
            let Some(writer) = w.as_mut() else {
                break Err(eio("Schreibkanal ist geschlossen"));
            };
            if let Err(e) = writer.write_all(&payload) {
                break Err(e);
            }
            continue;
        }
        if tag != TAG_CTRL {
            break Err(eio("unerwarteter Frame beim Schreiben"));
        }
        match serde_json::from_slice::<Ctrl>(&payload).map_err(eio) {
            Ok(Ctrl::Fs {
                req: FsRequest::WriteDone,
            }) => {
                let Some(mut writer) = w.take() else {
                    break Err(eio("Schreibkanal ist geschlossen"));
                };
                if let Err(e) = writer.flush() {
                    break Err(e);
                }
                drop(writer);
                break t.backend.rename(&tmp_path, &t.path);
            }
            Ok(_) => break Err(eio("unerwartete Steuernachricht beim Schreiben")),
            Err(e) => break Err(e),
        }
    };
    match result {
        Ok(()) => reply(send, FsResponse::Ok).await,
        Err(e) => {
            let msg = e.to_string();
            drop(w.take());
            let _ = t.backend.remove_file(&tmp_path);
            reply_err(send, eio(msg)).await
        }
    }
}

async fn reply(send: &mut SendStream, resp: FsResponse) -> io::Result<()> {
    send_ctrl(send, &Ctrl::FsResp { resp }).await
}

async fn reply_err(send: &mut SendStream, e: io::Error) -> io::Result<()> {
    reply(send, FsResponse::Err { msg: e.to_string() }).await
}

async fn send_ctrl(send: &mut SendStream, ctrl: &Ctrl) -> io::Result<()> {
    send_tagged(send, TAG_CTRL, &serde_json::to_vec(ctrl).map_err(eio)?).await
}

async fn recv_ctrl(recv: &mut RecvStream) -> io::Result<Ctrl> {
    let (tag, payload) = recv_tagged(recv).await?;
    if tag != TAG_CTRL {
        return Err(eio("Peer sendet keinen Steuerframe"));
    }
    serde_json::from_slice::<Ctrl>(&payload).map_err(eio)
}

async fn recv_resp(recv: &mut RecvStream) -> io::Result<FsResponse> {
    match recv_ctrl(recv).await? {
        Ctrl::FsResp {
            resp: FsResponse::Err { msg },
        } => Err(eio(msg)),
        Ctrl::FsResp { resp } => Ok(resp),
        _ => Err(eio("Peer sendet falsche Antwort")),
    }
}

async fn send_tagged(send: &mut SendStream, tag: u8, payload: &[u8]) -> io::Result<()> {
    let n = payload
        .len()
        .checked_add(1)
        .ok_or_else(|| eio("Frame zu gross"))?;
    if n > MAX_FRAME {
        return Err(eio("Frame zu gross"));
    }
    send.write_all(&(n as u32).to_be_bytes())
        .await
        .map_err(eio)?;
    send.write_all(&[tag]).await.map_err(eio)?;
    send.write_all(payload).await.map_err(eio)?;
    send.flush().await.map_err(eio)
}

async fn recv_tagged(recv: &mut RecvStream) -> io::Result<(u8, Vec<u8>)> {
    let mut len4 = [0u8; 4];
    recv.read_exact(&mut len4).await.map_err(eio)?;
    let n = u32::from_be_bytes(len4) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(eio("Frame zu gross"));
    }
    let mut buf = vec![0u8; n];
    recv.read_exact(&mut buf).await.map_err(eio)?;
    Ok((buf[0], buf[1..].to_vec()))
}

fn resolve_incoming_session(
    hello: &PeerHello,
    remote_node: &str,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> io::Result<ShareExportConfig> {
    if hello.node_id != remote_node {
        return Err(eio("Iroh NodeId passt nicht zum Session-Handshake"));
    }
    let state = auth.lock().map_err(|_| eio("Share-Auth gesperrt"))?.clone();
    match hello.relation_kind.as_str() {
        "direct" if hello.relation_id == state.identity.direct_lookup_id => {
            if !state.direct_online {
                return Err(eio("Direktverbindung ist offline"));
            }
            let grant = state
                .direct_grants
                .iter()
                .find(|g| {
                    g.device_id == hello.device_id
                        && g.state == DirectGrantState::Accepted
                        && g.public_key == hello.public_key
                        && g.node_id == hello.node_id
                })
                .ok_or_else(|| eio("Direktfreigabe nicht akzeptiert"))?;
            if !fingerprint_matches(&grant.public_key, &grant.fingerprint) {
                return Err(eio("Direktfreigabe hat ungueltigen Fingerprint"));
            }
            let payload = session_payload(
                "direct",
                &hello.relation_id,
                &hello.device_id,
                &state.identity.device_id,
                &hello.node_id,
                &state.identity.node_id,
                &hello.session_nonce,
            );
            if !verify_hmac(&state.direct_secret, &payload, &hello.session_proof) {
                return Err(eio("Session-Proof ungueltig"));
            }
            return Ok(state.default_direct_exports);
        }
        "room" => {
            let room = state
                .rooms
                .iter()
                .find(|r| r.room_id == hello.relation_id)
                .cloned()
                .ok_or_else(|| eio("Unbekannter Raum"))?;
            let member = room
                .members
                .iter()
                .find(|m| m.device_id == hello.device_id && !m.blocked)
                .ok_or_else(|| eio("Geraet nicht im Raum"))?;
            if member.node_id != hello.node_id || member.public_key != hello.public_key {
                return Err(eio("Raumgeraet hat Identitaetskonflikt"));
            }
            let secret =
                ShareProfiles::room_secret(&room).ok_or_else(|| eio("Raum-Secret fehlt"))?;
            let payload = session_payload(
                "room",
                &hello.relation_id,
                &hello.device_id,
                &state.identity.device_id,
                &hello.node_id,
                &state.identity.node_id,
                &hello.session_nonce,
            );
            if !verify_hmac(&secret, &payload, &hello.session_proof) {
                return Err(eio("Session-Proof ungueltig"));
            }
            return Ok(room.exports);
        }
        _ => return Err(eio("Unbekannte oder nicht autorisierte Relation")),
    }
}

fn endpoint_addr(p: &super::types::PeerPresence) -> io::Result<EndpointAddr> {
    let node: EndpointId = p.node_id.parse().map_err(eio)?;
    let mut addrs: Vec<TransportAddr> = p
        .candidates
        .iter()
        .filter_map(|c| c.parse::<SocketAddr>().ok())
        .map(TransportAddr::Ip)
        .collect();
    if let Ok(relay) = p.relay_url.parse::<RelayUrl>() {
        addrs.push(TransportAddr::Relay(relay));
    }
    Ok(EndpointAddr::from_parts(node, addrs))
}

fn transport_label(conn: &Connection) -> &'static str {
    let paths = conn.paths();
    paths
        .iter()
        .find(|p| p.is_selected())
        .map(|p| if p.is_relay() { "relay" } else { "direct" })
        .unwrap_or("unknown")
}

fn fs_request_label(req: &FsRequest) -> &'static str {
    match req {
        FsRequest::ListDir { .. } => "list_dir",
        FsRequest::Stat { .. } => "stat",
        FsRequest::Read { .. } => "read",
        FsRequest::Write { .. } => "write",
        FsRequest::WriteDone => "write_done",
        FsRequest::MkdirAll { .. } => "mkdir_all",
        FsRequest::Rename { .. } => "rename",
        FsRequest::CopyFile { .. } => "copy_file",
        FsRequest::RemoveFile { .. } => "remove_file",
        FsRequest::RemoveDir { .. } => "remove_dir",
    }
}

fn fs_response_summary(resp: &FsResponse) -> String {
    match resp {
        FsResponse::Entries { entries } => format!("{} Eintraege", entries.len()),
        FsResponse::Meta { meta } => format!("meta size={} dir={}", meta.size, meta.is_dir),
        FsResponse::Data { size } => format!("{size} bytes"),
        FsResponse::Ready => "bereit".into(),
        FsResponse::Ok => "ok".into(),
        FsResponse::Err { msg } => format!("fehler={msg}"),
    }
}

fn relation_kind_id(endpoint: &PeerEndpoint) -> (&'static str, String) {
    match &endpoint.scope {
        ShareScope::Direct { .. } => ("direct", endpoint.presence.relation_id.clone()),
        ShareScope::Room { room_id } => ("room", room_id.clone()),
    }
}

fn session_key(endpoint: &PeerEndpoint) -> String {
    let (kind, relation_id) = relation_kind_id(endpoint);
    format!("{kind}:{relation_id}:{}", endpoint.presence.node_id)
}

fn session_payload(
    kind: &str,
    relation_id: &str,
    from_device: &str,
    to_device: &str,
    from_node: &str,
    to_node: &str,
    nonce: &str,
) -> String {
    format!(
        "smart-explorer/share/session/v3|{kind}|{relation_id}|{from_device}|{to_device}|{from_node}|{to_node}|{nonce}"
    )
}

fn relay_url_from_signal(config: &str) -> String {
    if let Ok(url) = std::env::var("SE_SHARE_RELAY_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return trim_url_path(trimmed);
        }
    }
    let first = config
        .split([',', ';'])
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("127.0.0.1:51820");
    let normalized = if let Some(rest) = first.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = first.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if first.starts_with("https://") || first.starts_with("http://") {
        first.to_string()
    } else if let Some(rest) = first.strip_prefix("tcp://") {
        format!("http://{}", relay_tcp_addr(rest))
    } else if first.contains("://") {
        first.to_string()
    } else {
        format!("http://{}", relay_tcp_addr(first))
    };
    trim_url_path(&normalized)
}

fn relay_tcp_addr(addr: &str) -> String {
    let addr = normalize_tcp_addr(addr);
    if let Ok(mut socket) = addr.parse::<SocketAddr>() {
        socket.set_port(socket.port().saturating_add(1));
        return socket.to_string();
    }
    if let Some((host, port)) = split_host_port(&addr) {
        if let Ok(port) = port.parse::<u16>() {
            return format!("{host}:{}", port.saturating_add(1));
        }
    }
    addr
}

fn split_host_port(addr: &str) -> Option<(&str, &str)> {
    let (host, port) = addr.rsplit_once(':')?;
    if port.contains(']') {
        return None;
    }
    Some((host, port))
}

fn normalize_tcp_addr(addr: &str) -> String {
    let addr = addr.trim().trim_end_matches('/');
    if addr.is_empty() || addr.starts_with('[') || addr.rsplit_once(':').is_some() {
        addr.to_string()
    } else {
        format!("{addr}:51820")
    }
}

fn trim_url_path(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let authority = rest.split('/').next().unwrap_or(rest);
        format!("{scheme}://{authority}")
    } else {
        url.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::core::public_fingerprint;
    use crate::share::fs::SharedRoot;
    use crate::share::types::PeerPresence;
    use crate::vfs::Backend;
    use crossbeam_channel::unbounded;
    use std::fs;
    use std::io::{Read, Write};

    #[test]
    fn iroh_direct_session_transfers_files() {
        let secret = vec![7u8; 32];
        let root = std::env::temp_dir().join(format!(
            "se-iroh-direct-{}-{}",
            std::process::id(),
            crate::share::core_now_secs()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("hello.txt"), b"hello from gate").unwrap();

        let a = test_identity("device-a", "Device A", "lookup-a");
        let b = test_identity("device-b", "Device B", "lookup-b");
        let (tx_a, _events_a) = unbounded();
        let (tx_b, _events_b) = unbounded();

        let auth_b = Arc::new(Mutex::new(ShareAuthState {
            identity: b.clone(),
            direct_secret: secret.clone(),
            default_direct_exports: ShareExportConfig {
                roots: vec![SharedRoot {
                    label: "Gate".into(),
                    path: root.to_string_lossy().replace('\\', "/"),
                }],
                include_connections: false,
            },
            direct_contacts: Vec::new(),
            direct_grants: vec![crate::share::types::DirectGrant {
                device_id: a.device_id.clone(),
                device_name: a.device_name.clone(),
                public_key: a.public_key.clone(),
                fingerprint: a.fingerprint.clone(),
                node_id: a.node_id.clone(),
                state: DirectGrantState::Accepted,
                updated_at: 1,
            }],
            rooms: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
        }));
        let auth_a = Arc::new(Mutex::new(ShareAuthState {
            identity: a.clone(),
            direct_secret: vec![0u8; 32],
            default_direct_exports: ShareExportConfig::default(),
            direct_contacts: Vec::new(),
            direct_grants: Vec::new(),
            rooms: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
        }));

        let node_b = ShareIrohNode::start("tcp://127.0.0.1:51820", &b, auth_b, tx_b).unwrap();
        let node_a = ShareIrohNode::start("tcp://127.0.0.1:51820", &a, auth_a, tx_a).unwrap();
        let presence = PeerPresence {
            kind: "direct".into(),
            relation_id: b.direct_lookup_id.clone(),
            device_id: b.device_id.clone(),
            device_name: b.device_name.clone(),
            public_key: b.public_key.clone(),
            fingerprint: b.fingerprint.clone(),
            node_id: b.node_id.clone(),
            relay_url: String::new(),
            candidates: node_b.candidates(),
            expires_at: crate::share::core_now_secs() + 300,
            nonce: "test".into(),
            proof: String::new(),
        };
        let endpoint = PeerEndpoint {
            label: "test".into(),
            scope: ShareScope::Direct {
                contact_id: "contact-b".into(),
            },
            presence,
            relation_secret: secret,
            expected_node_id: Some(b.node_id.clone()),
        };
        let backend = PeerBackend::new(endpoint, a.clone(), node_a);

        let root_entries = backend.list_dir("/").unwrap();
        assert!(root_entries.iter().any(|e| e.name == "Gate" && e.is_dir));
        let mut text = String::new();
        backend
            .open_read("/Gate/hello.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "hello from gate");

        backend
            .open_write("/Gate/new.txt")
            .unwrap()
            .write_all(b"written over iroh")
            .unwrap();
        assert_eq!(
            fs::read(root.join("new.txt")).unwrap(),
            b"written over iroh"
        );
        assert_eq!(
            backend
                .copy_file("/Gate/new.txt", "/Gate/copy.txt")
                .unwrap(),
            17
        );
        backend
            .rename("/Gate/copy.txt", "/Gate/renamed.txt")
            .unwrap();
        assert!(root.join("renamed.txt").exists());
        backend.remove_file("/Gate/renamed.txt").unwrap();
        assert!(!root.join("renamed.txt").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn relay_url_tracks_signal_transport() {
        assert_eq!(
            relay_url_from_signal("tcp://127.0.0.1:51820"),
            "http://127.0.0.1:51821"
        );
        assert_eq!(
            relay_url_from_signal("127.0.0.1:51820"),
            "http://127.0.0.1:51821"
        );
        assert_eq!(
            relay_url_from_signal("wss://share.example/se-share"),
            "https://share.example"
        );
        assert_eq!(
            relay_url_from_signal("https://share.example/se-share"),
            "https://share.example"
        );
    }

    fn test_identity(device_id: &str, device_name: &str, lookup: &str) -> ShareIdentity {
        let secret = iroh::SecretKey::generate();
        let node_id = secret.public().to_string();
        let fingerprint = public_fingerprint(node_id.as_bytes());
        ShareIdentity {
            device_id: device_id.into(),
            device_name: device_name.into(),
            direct_lookup_id: lookup.into(),
            public_key: node_id.clone(),
            fingerprint,
            node_id,
            iroh_secret: secret,
        }
    }
}
