use std::io::{self, Read, Write};

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};

use super::core::{b64_decode, eio, public_fingerprint, random_token, relation_psk, room_psk};
use super::identity::ShareIdentity;
use super::protocol::{write_raw_frame, Channel, TAG_CTRL, TAG_DATA};
use super::relay::{send_relay_request, RelayStream};
use super::transfer::dial_candidates;
use super::types::{PeerEndpoint, ShareScope};
use super::wire::{Ctrl, FsMeta, FsRequest, FsResponse, PeerHello, PeerPrelude};

pub struct PeerBackend {
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
}

impl PeerBackend {
    pub(crate) fn new(endpoint: PeerEndpoint, identity: ShareIdentity) -> Self {
        Self { endpoint, identity }
    }

    pub(crate) fn probe_root(&self) -> io::Result<Vec<VfsMeta>> {
        self.list_dir("/")
    }

    fn relation_kind_id(&self) -> (&'static str, String) {
        match &self.endpoint.scope {
            ShareScope::Direct { .. } => ("direct", self.endpoint.presence.relation_id.clone()),
            ShareScope::Room { room_id } => ("room", room_id.clone()),
        }
    }

    fn psk(&self) -> [u8; 32] {
        match &self.endpoint.scope {
            ShareScope::Direct { .. } => relation_psk(
                "direct",
                &self.endpoint.relation_secret,
                &self.identity.device_id,
                &self.endpoint.presence.device_id,
            ),
            ShareScope::Room { room_id } => room_psk(&self.endpoint.relation_secret, room_id),
        }
    }

    fn channel(&self) -> io::Result<Channel> {
        if self.endpoint.presence.expires_at < super::core::now_secs() {
            return Err(eio("Peer-Presence ist abgelaufen"));
        }
        let expected_public = b64_decode(&self.endpoint.presence.public_key).map_err(eio)?;
        if public_fingerprint(&expected_public) != self.endpoint.presence.fingerprint {
            return Err(eio("Presence-Fingerprint passt nicht zum Public Key"));
        }
        if let Some(pinned) = &self.endpoint.expected_public_key {
            if pinned != &expected_public {
                return Err(eio(
                    "Identitaetskonflikt: Presence passt nicht zum gepinnten Key",
                ));
            }
        }
        let (kind, relation_id) = self.relation_kind_id();
        let prelude = PeerPrelude {
            relation_kind: kind.to_string(),
            relation_id: relation_id.clone(),
            from_device_id: self.identity.device_id.clone(),
        };
        let direct_attempt = dial_candidates(&self.endpoint.presence.candidates).and_then(|s| {
            self.open_channel_on_stream(s, &prelude, kind, relation_id.clone(), &expected_public)
        });
        match direct_attempt {
            Ok(ch) => return Ok(ch),
            Err(direct_error) => {
                let relay_id = random_token(16);
                send_relay_request(
                    &self.endpoint.server,
                    &self.identity,
                    &relay_id,
                    kind,
                    &relation_id,
                    &self.endpoint.presence.device_id,
                    &self.endpoint.relation_secret,
                )
                .map_err(|e| eio(format!("{direct_error}; Relay-Anfrage: {e}")))?;
                let relay = RelayStream::connect(
                    &self.endpoint.server,
                    &relay_id,
                    &self.identity.device_id,
                )
                .map_err(|e| eio(format!("{direct_error}; Relay verbinden: {e}")))?;
                let ch = self
                    .open_channel_on_stream(relay, &prelude, kind, relation_id, &expected_public)
                    .map_err(|e| eio(format!("{direct_error}; Relay-Kanal: {e}")))?;
                return Ok(ch);
            }
        }
    }

    fn open_channel_on_stream<S: super::protocol::IoStream + 'static>(
        &self,
        mut stream: S,
        prelude: &PeerPrelude,
        kind: &str,
        relation_id: String,
        expected_public: &[u8],
    ) -> io::Result<Channel> {
        write_raw_frame(&mut stream, &serde_json::to_vec(&prelude).map_err(eio)?)?;
        let mut ch = Channel::initiator(
            stream,
            &self.psk(),
            &self.identity.private_key,
            Some(&expected_public),
        )?;
        let hello = PeerHello {
            protocol_version: 2,
            relation_kind: kind.to_string(),
            relation_id,
            device_id: self.identity.device_id.clone(),
            public_key: self.identity.public_key.clone(),
            requested_capabilities: vec!["fs".to_string()],
        };
        send_ctrl(&mut ch, Ctrl::PeerHello { hello })?;
        match recv_ctrl(&mut ch)? {
            Ctrl::PeerHelloOk => Ok(ch),
            Ctrl::FsResp {
                resp: FsResponse::Err { msg },
            } => Err(eio(msg)),
            _ => Err(eio("Peer akzeptiert den sicheren Kanal nicht")),
        }
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
    send_ctrl(ch, Ctrl::Fs { req })
}

fn send_ctrl(ch: &mut Channel, ctrl: Ctrl) -> io::Result<()> {
    ch.send(TAG_CTRL, &serde_json::to_vec(&ctrl).map_err(eio)?)
}

fn recv_ctrl(ch: &mut Channel) -> io::Result<Ctrl> {
    let (tag, payload) = ch.recv()?;
    if tag != TAG_CTRL {
        return Err(eio("Peer sendet keinen Steuerframe"));
    }
    serde_json::from_slice::<Ctrl>(&payload).map_err(eio)
}

fn recv_resp(ch: &mut Channel) -> io::Result<FsResponse> {
    match recv_ctrl(ch)? {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::collections::{HashMap, HashSet};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::share::core::{
        b64, b64_decode, hmac_proof, now_secs, presence_payload, public_fingerprint, random_bytes,
        random_token,
    };
    use crate::share::fs::{ShareExportConfig, SharedRoot};
    use crate::share::identity::ShareIdentity;
    use crate::share::relay::RelayStream;
    use crate::share::transfer::{accept_loop, recv_from_peer, ShareAuthState};
    use crate::share::types::{
        DirectGrant, DirectGrantState, PeerPresence, ShareEvent, ShareScope,
    };
    use crate::share::wire::ClientMsg;
    use crate::vfs::Backend;

    #[test]
    fn direct_peer_backend_opens_folder_and_transfers_files() {
        let fixture = Fixture::new("direct");
        let stopped = Arc::new(AtomicBool::new(false));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let candidate = listener.local_addr().unwrap().to_string();
        let auth = fixture.auth_state();
        let (ev_tx, _ev_rx) = unbounded::<ShareEvent>();
        let accept_stopped = stopped.clone();
        std::thread::spawn(move || accept_loop(listener, auth, ev_tx, accept_stopped));

        let backend = fixture.backend(vec![candidate], "127.0.0.1:1");
        exercise_peer_backend(&backend, &fixture.root);
        stopped.store(true, Ordering::Relaxed);
        fixture.cleanup();
    }

    #[test]
    fn relay_peer_backend_opens_folder_and_transfers_files_when_direct_fails() {
        let fixture = Fixture::new("relay");
        let relay_stop = Arc::new(AtomicBool::new(false));
        let (relay_server, relay_rx) = start_test_relay_server(relay_stop.clone());
        let responder_stop = Arc::new(AtomicBool::new(false));
        let responder = start_relay_responder(
            relay_server.clone(),
            relay_rx,
            fixture.b.clone(),
            fixture.auth_state(),
            responder_stop.clone(),
        );

        let backend = fixture.backend(vec!["not-a-socket-address".into()], &relay_server);
        exercise_peer_backend(&backend, &fixture.root);

        responder_stop.store(true, Ordering::Relaxed);
        relay_stop.store(true, Ordering::Relaxed);
        let _ = responder.join();
        fixture.cleanup();
    }

    struct Fixture {
        a: ShareIdentity,
        b: ShareIdentity,
        direct_secret: Vec<u8>,
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let a = test_identity(&format!("{name}-a"), "device-a", "lookup-a");
            let b = test_identity(&format!("{name}-b"), "device-b", "lookup-b");
            let direct_secret = random_bytes::<32>().to_vec();
            let root = std::env::temp_dir().join(format!(
                "se-share-peer-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("source.txt"), b"hello from share").unwrap();
            Self {
                a,
                b,
                direct_secret,
                root,
            }
        }

        fn auth_state(&self) -> Arc<Mutex<ShareAuthState>> {
            Arc::new(Mutex::new(ShareAuthState {
                identity: self.b.clone(),
                direct_secret: self.direct_secret.clone(),
                default_direct_exports: ShareExportConfig {
                    roots: vec![SharedRoot {
                        label: "Gate".into(),
                        path: self.root.to_string_lossy().replace('\\', "/"),
                    }],
                    include_connections: false,
                },
                direct_contacts: Vec::new(),
                direct_grants: vec![DirectGrant {
                    device_id: self.a.device_id.clone(),
                    device_name: self.a.device_name.clone(),
                    public_key: self.a.public_key.clone(),
                    fingerprint: self.a.fingerprint.clone(),
                    state: DirectGrantState::Accepted,
                    updated_at: now_secs(),
                }],
                rooms: Vec::new(),
                seen_nonces: HashSet::new(),
                direct_online: true,
            }))
        }

        fn backend(&self, candidates: Vec<String>, server: &str) -> PeerBackend {
            let expected_public_key = b64_decode(&self.b.public_key).ok();
            PeerBackend::new(
                PeerEndpoint {
                    label: "Share Test".into(),
                    scope: ShareScope::Direct {
                        contact_id: "contact-b".into(),
                    },
                    presence: direct_presence(&self.b, &self.direct_secret, candidates),
                    relation_secret: self.direct_secret.clone(),
                    expected_public_key,
                    server: server.to_string(),
                },
                self.a.clone(),
            )
        }

        fn cleanup(&self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn exercise_peer_backend(backend: &PeerBackend, root: &std::path::Path) {
        let root_entries = backend.list_dir("/").unwrap();
        assert!(root_entries.iter().any(|e| e.name == "Gate" && e.is_dir));
        let gate_entries = backend.list_dir("/Gate").unwrap();
        assert!(gate_entries.iter().any(|e| e.name == "source.txt"));

        let mut text = String::new();
        let mut reader = backend.open_read("/Gate/source.txt").unwrap();
        reader.read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello from share");

        backend.mkdir_all("/Gate/sub").unwrap();
        {
            let mut writer = backend.open_write("/Gate/sub/written.txt").unwrap();
            writer.write_all(b"written through peer").unwrap();
            writer.flush().unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(root.join("sub").join("written.txt")).unwrap(),
            "written through peer"
        );
        backend
            .rename("/Gate/sub/written.txt", "/Gate/sub/renamed.txt")
            .unwrap();
        assert!(root.join("sub").join("renamed.txt").exists());
        assert_eq!(
            backend
                .copy_file("/Gate/source.txt", "/Gate/copied.txt")
                .unwrap(),
            "hello from share".len() as u64
        );
        assert_eq!(
            std::fs::read_to_string(root.join("copied.txt")).unwrap(),
            "hello from share"
        );
        backend.remove_file("/Gate/copied.txt").unwrap();
        backend.remove_dir("/Gate/sub").unwrap();
        assert!(!root.join("copied.txt").exists());
        assert!(!root.join("sub").exists());
    }

    fn test_identity(name: &str, device_id: &str, lookup_id: &str) -> ShareIdentity {
        let params = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s".parse().unwrap();
        let kp = snow::Builder::new(params).generate_keypair().unwrap();
        let public_key = b64(&kp.public);
        ShareIdentity {
            device_id: device_id.into(),
            device_name: name.into(),
            direct_lookup_id: lookup_id.into(),
            fingerprint: public_fingerprint(&kp.public),
            public_key,
            private_key: kp.private,
        }
    }

    fn direct_presence(
        identity: &ShareIdentity,
        secret: &[u8],
        candidates: Vec<String>,
    ) -> PeerPresence {
        let expires_at = now_secs() + 90;
        let nonce = random_token(12);
        let payload = presence_payload(
            "direct",
            &identity.direct_lookup_id,
            &identity.device_id,
            &identity.public_key,
            &candidates,
            expires_at,
            &nonce,
        );
        PeerPresence {
            kind: "direct".into(),
            relation_id: identity.direct_lookup_id.clone(),
            device_id: identity.device_id.clone(),
            device_name: identity.device_name.clone(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
            candidates,
            expires_at,
            nonce,
            proof: hmac_proof(secret, &payload),
        }
    }

    fn start_relay_responder(
        server: String,
        relay_rx: mpsc::Receiver<String>,
        identity: ShareIdentity,
        auth: Arc<Mutex<ShareAuthState>>,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut workers = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                match relay_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(relay_id) => {
                        let server = server.clone();
                        let device_id = identity.device_id.clone();
                        let auth = auth.clone();
                        workers.push(std::thread::spawn(move || {
                            let stream = RelayStream::connect(&server, &relay_id, &device_id)
                                .expect("relay responder connects");
                            let (ev_tx, _ev_rx) = unbounded::<ShareEvent>();
                            recv_from_peer(stream, 0, auth, &ev_tx).expect("relay fs request");
                        }));
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            for worker in workers {
                let _ = worker.join();
            }
        })
    }

    fn start_test_relay_server(stop: Arc<AtomicBool>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let pending = Arc::new(Mutex::new(HashMap::<String, TcpStream>::new()));
        let (tx, rx) = mpsc::channel::<String>();
        let server_stop = stop.clone();
        std::thread::spawn(move || {
            while !server_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let pending = pending.clone();
                        let tx = tx.clone();
                        std::thread::spawn(move || handle_test_relay_conn(stream, pending, tx));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (addr, rx)
    }

    fn handle_test_relay_conn(
        mut stream: TcpStream,
        pending: Arc<Mutex<HashMap<String, TcpStream>>>,
        relay_tx: mpsc::Sender<String>,
    ) {
        let first = match read_test_line_raw(&mut stream) {
            Ok(line) if !line.is_empty() => line,
            _ => return,
        };
        let msg = match serde_json::from_str::<ClientMsg>(first.trim()) {
            Ok(msg) => msg,
            Err(_) => return,
        };
        match msg {
            ClientMsg::RelayJoin { relay_id, .. } => {
                let peer = {
                    let mut pending = pending.lock().unwrap();
                    if let Some(peer) = pending.remove(&relay_id) {
                        Some(peer)
                    } else {
                        pending.insert(relay_id, stream.try_clone().unwrap());
                        None
                    }
                };
                if let Some(peer) = peer {
                    let _ = bridge_test_streams(peer, stream);
                }
            }
            ClientMsg::Hello { .. } => {
                let second = match read_test_line_raw(&mut stream) {
                    Ok(line) if !line.is_empty() => line,
                    _ => return,
                };
                if let Ok(ClientMsg::RelayRequest { relay_id, .. }) =
                    serde_json::from_str::<ClientMsg>(second.trim())
                {
                    let _ = relay_tx.send(relay_id);
                }
            }
            _ => {}
        }
    }

    fn read_test_line_raw(stream: &mut TcpStream) -> io::Result<String> {
        let mut out = String::new();
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => return Ok(out),
                Ok(1) => {
                    if byte[0] == b'\n' {
                        return Ok(out);
                    }
                    out.push(byte[0] as char);
                }
                Ok(_) => unreachable!(),
                Err(e) => return Err(e),
            }
        }
    }

    fn bridge_test_streams(mut a: TcpStream, mut b: TcpStream) -> io::Result<()> {
        a.set_read_timeout(Some(Duration::from_millis(100)))?;
        b.set_read_timeout(Some(Duration::from_millis(100)))?;
        let started = std::time::Instant::now();
        let mut last_data = std::time::Instant::now();
        loop {
            let moved_a = pump_test_stream(&mut a, &mut b)?;
            let moved_b = pump_test_stream(&mut b, &mut a)?;
            if moved_a || moved_b {
                last_data = std::time::Instant::now();
                continue;
            }
            if last_data.elapsed() > Duration::from_secs(30)
                || started.elapsed() > Duration::from_secs(120)
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn pump_test_stream(from: &mut TcpStream, to: &mut TcpStream) -> io::Result<bool> {
        let mut buf = [0u8; 16 * 1024];
        match from.read(&mut buf) {
            Ok(0) => Ok(false),
            Ok(n) => {
                to.write_all(&buf[..n])?;
                to.flush()?;
                Ok(true)
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}
