//! Unprivileged machine endpoint. The independent local broker owns every effect.
use std::{io, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use iroh::endpoint::Connection;
use sha2::{Digest, Sha256};
use tokio::{sync::{mpsc, oneshot, watch, Semaphore}, task::{JoinHandle, JoinSet}};
use zeroize::Zeroizing;
use crate::{login::{core::{gateway_types::{ClientHello, GatewayAccess, GatewayNetwork, GatewayReply, GatewayTarget, ServerHello, LOGIN_ALPN}, gateway_wire as wire,
    machine::MachineIdentity, model::{Challenge, Operation, Policy, SignedRequest, VERSION}, wire::Reply}, os::platform}, share::login_transport as transport};
use super::{client::GatewayClient, denied, installation, now};
const HANDSHAKE: Duration = Duration::from_secs(30);
const BROKER_IDLE: Duration = Duration::from_secs(5);
// The existing Share endpoint permits 64 pending application handshakes. The
// login broker deliberately admits one active console session at a time.
const PENDING_HANDSHAKES: usize = 64;
#[derive(Clone)]
struct Epoch { policy: Arc<Policy>, stamp: [u8; 32] }
fn epoch(policy: Policy) -> io::Result<Epoch> {
    let stamp = Sha256::digest(serde_json::to_vec(&policy).map_err(io::Error::other)?).into();
    Ok(Epoch { policy: Arc::new(policy), stamp })
}

pub(crate) fn run_until(stop: Arc<AtomicBool>, ready: impl FnOnce()) -> io::Result<()> {
    run_until_ready(stop, || { ready(); Ok(()) })
}
pub(crate) fn run_until_ready(stop: Arc<AtomicBool>, ready: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let _install = installation::running()?;
    let policy = installation::policy()?; platform::require_gateway(&policy)?;
    let raw = Zeroizing::new(platform::read_gateway_key()?);
    let identity: MachineIdentity = serde_json::from_slice(&raw).map_err(|_| denied("private gateway machine identity JSON is invalid"))?;
    let key = identity.key_for_policy(&policy)?;
    let options: GatewayNetwork = match platform::read_gateway_network()? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| denied("private gateway network JSON is invalid"))?, None => GatewayNetwork::default(),
    };
    options.validate()?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("login-gateway").build()?;
    let result = runtime.block_on(serve(key, identity.machine_id.clone(), options, epoch(policy)?, stop, ready));
    // Blocking broker operations have bounded native IPC. Dropping their command
    // sender cooperatively ends the worker; abort is never treated as cancellation.
    runtime.shutdown_timeout(Duration::from_secs(15)); result
}
async fn serve(key: iroh::SecretKey, machine: String, options: GatewayNetwork, initial: Epoch,
    stop: Arc<AtomicBool>, ready: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let endpoint = transport::bind_machine(key, &options).await?;
    let (epochs, _) = watch::channel(initial);
    let pending = Arc::new(Semaphore::new(PENDING_HANDSHAKES));
    let sessions = Arc::new(Semaphore::new(1));
    let mut children = JoinSet::new();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut published = Vec::new();
    let address = serde_json::to_vec(&transport::address(&machine, &endpoint, &options)).map_err(io::Error::other)?;
    platform::write_gateway_address(&address)?; ready()?;
    let result = (async {
        loop {
            tokio::select! {
                biased;
                _ = tick.tick() => {
                    if stop.load(Ordering::SeqCst) { break; }
                    let next = tokio::task::spawn_blocking(installation::policy).await.map_err(io::Error::other)??;
                    platform::require_gateway(&next)?;
                    if next.machine_id != machine || next.machine_public_key != endpoint.id().to_string() {
                        return Err(denied("administratively approved machine identity changed; restart the gateway with its matching private key"));
                    }
                    let next = epoch(next)?;
                    if next.stamp != epochs.borrow().stamp { epochs.send_replace(next); }
                    let address = serde_json::to_vec(&transport::address(&machine, &endpoint, &options)).map_err(io::Error::other)?;
                    if address != published { let copy = address.clone(); tokio::task::spawn_blocking(move || platform::write_gateway_address(&copy)).await.map_err(io::Error::other)??; published = address; }
                }
                Some(_) = children.join_next(), if !children.is_empty() => {},
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else { break; };
                    let Ok(slot) = pending.clone().try_acquire_owned() else { incoming.refuse(); continue; };
                    let epochs = epochs.subscribe(); let sessions = sessions.clone(); let local = endpoint.id().to_string();
                    children.spawn(async move {
                        let _slot = slot;
                        let connection = tokio::time::timeout(HANDSHAKE, incoming).await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "login TLS handshake timed out"))?.map_err(io::Error::other)?;
                        handle(connection, local, epochs, sessions).await
                    });
                }
            }
        }
        Ok(())
    }).await;
    // Closing the endpoint revokes transport admission before async tasks drop
    // their local broker senders and release the helper through local EOF.
    endpoint.close().await; children.shutdown().await; result
}
async fn guarded<T>(connection: &Connection, epochs: &mut watch::Receiver<Epoch>, active: Duration,
    deadline: Instant, future: impl std::future::Future<Output = io::Result<T>>) -> io::Result<T> {
    tokio::select! {
        biased;
        _ = epochs.changed() => Err(denied("administrative login policy changed")),
        _ = connection.closed() => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "remote login transport closed")),
        result = transport::routed(connection, active, deadline, future) => result,
    }
}
async fn handle(connection: Connection, local: String, mut epochs: watch::Receiver<Epoch>, sessions: Arc<Semaphore>) -> io::Result<()> {
    let _close = transport::Close(connection.clone());
    if connection.alpn() != LOGIN_ALPN { return Err(denied("unexpected login ALPN")); }
    let remote = connection.remote_id().to_string();
    let initial = epochs.borrow_and_update().clone();
    let authorized = initial.policy.grants.iter().any(|grant| grant.public_key == remote && grant.rights.login && grant.rights.screen_view
        && now().is_ok_and(|now| now >= grant.not_before && now < grant.expires_at));
    transport::monitor(&connection, local, true, Vec::new())?;
    let deadline = Instant::now() + HANDSHAKE;
    let (mut send, mut recv) = guarded(&connection, &mut epochs, HANDSHAKE, deadline, async { connection.open_bi().await.map_err(io::Error::other) }).await?;
    let server = ServerHello { format: VERSION, machine_id: initial.policy.machine_id.clone(), public_key: initial.policy.machine_public_key.clone(), route_policy: VERSION,
        admission: if authorized { "ready" } else { "permission_missing" }.into() };
    guarded(&connection, &mut epochs, HANDSHAKE, deadline, wire::send(&mut send, &server)).await?;
    if !authorized { let _ = tokio::time::timeout(BROKER_IDLE, connection.closed()).await; return Ok(()); }
    let hello: ClientHello = guarded(&connection, &mut epochs, HANDSHAKE, deadline, wire::receive(&mut recv)).await?;
    let grant = match hello.authorize(&initial.policy, &remote, now()?) {
        Ok(grant) => grant,
        Err(error) => {
            let refused = GatewayReply { request_sequence: 0, reply: Reply::refused_with("permission_missing", error), access: None };
            guarded(&connection, &mut epochs, BROKER_IDLE, Instant::now() + BROKER_IDLE, wire::send_reply(&mut send, &refused, &[])).await?;
            let _ = tokio::time::timeout(BROKER_IDLE, connection.closed()).await; return Ok(());
        }
    };
    let expires_at = grant.expires_at;
    let access = GatewayAccess { peer_id: grant.peer_id.clone(), public_key: grant.public_key.clone(),
        target_user: hello.target_user.clone(), policy_revision: initial.policy.revision,
        not_before: grant.not_before, expires_at, rights: grant.rights.clone() };
    let remaining = std::time::UNIX_EPOCH.checked_add(Duration::from_secs(expires_at))
        .ok_or_else(|| denied("login grant expiry exceeds the platform clock"))?
        .duration_since(std::time::SystemTime::now()).map_err(|_| denied("login grant expired during authentication"))?;
    let grant_deadline = Instant::now().checked_add(remaining)
        .ok_or_else(|| denied("login grant expiry cannot be represented as a bounded monotone deadline"))?;
    transport::authenticated(&connection);
    let Ok(_session) = sessions.try_acquire_owned() else {
        let busy = GatewayReply { request_sequence: 0, reply: Reply::refused_with("busy", "another authorized login session owns the active console"), access: None };
        guarded(&connection, &mut epochs, BROKER_IDLE, Instant::now() + BROKER_IDLE, wire::send_reply(&mut send, &busy, &[])).await?;
        let _ = tokio::time::timeout(BROKER_IDLE, connection.closed()).await;
        return Ok(());
    };
    let mut worker = BrokerWorker::start(hello.peer_id.clone(), hello.target_user.clone());
    let result = (async {
        let mut challenge = match guarded(&connection, &mut epochs, BROKER_IDLE, (Instant::now() + BROKER_IDLE).min(grant_deadline), worker.ready()).await {
            Ok(challenge) => challenge,
            Err(error) => {
                let state = if error.kind() == io::ErrorKind::NotFound { "installation_required" } else { "unavailable" };
                let refused = GatewayReply { request_sequence: 0, reply: Reply::refused_with(state, error), access: None };
                guarded(&connection, &mut epochs, BROKER_IDLE, (Instant::now() + BROKER_IDLE).min(grant_deadline), wire::send_reply(&mut send, &refused, &[])).await?;
                let _ = tokio::time::timeout(BROKER_IDLE, connection.closed()).await; return Ok(());
            }
        };
        let pin = GatewayTarget { format: VERSION, machine_id: server.machine_id.clone(), public_key: server.public_key.clone(), direct_addresses: Vec::new(), relay_urls: Vec::new(), use_default_relays: true, allow_plaintext_relays: false };
        pin.check_challenge(&challenge, &hello.target_user)?;
        let first = GatewayReply { request_sequence: 0, reply: Reply { challenge: Some(challenge.clone()), ..Reply::state("challenge") }, access: Some(access) };
        guarded(&connection, &mut epochs, BROKER_IDLE, (Instant::now() + BROKER_IDLE).min(grant_deadline), wire::send_reply(&mut send, &first, &[])).await?;
        loop {
            let request: SignedRequest = guarded(&connection, &mut epochs, BROKER_IDLE, (Instant::now() + BROKER_IDLE).min(grant_deadline), wire::receive(&mut recv)).await?;
            hello.authorize(&epochs.borrow().policy, &remote, now()?)?;
            hello.check_operation(&request, &remote, &challenge)?;
            let sequence = request.challenge.sequence;
            let stop = matches!(request.operation, Operation::Stop);
            // Anchor the entire response to the existing broker idle budget.
            // Compression/path backpressure never grants an unsigned extension
            // while the broker is already waiting for the next signed request.
            let response_deadline = (Instant::now() + BROKER_IDLE).min(grant_deadline);
            let (reply, pixels) = guarded(&connection, &mut epochs, BROKER_IDLE, response_deadline, worker.ask(request)).await?;
            if let Some(next) = &reply.challenge { pin.check_challenge(next, &hello.target_user)?; }
            let packet = GatewayReply { request_sequence: sequence, reply, access: None };
            // PNG encoding runs only in this unprivileged gateway, with one
            // bounded independent frame; no baseline survives a context change.
            let encoded = guarded(&connection, &mut epochs, BROKER_IDLE, response_deadline, wire::prepare_reply(packet, pixels, worker.images()?)).await?;
            guarded(&connection, &mut epochs, BROKER_IDLE, response_deadline, wire::send_prepared(&mut send, &encoded)).await?;
            if stop || encoded.packet.reply.challenge.is_none() {
                let _ = tokio::time::timeout(BROKER_IDLE, connection.closed()).await;
                return Ok(());
            }
            challenge = encoded.packet.reply.challenge.ok_or_else(|| denied("broker returned no next challenge"))?;
        }
    }).await;
    transport::close(&connection); worker.finish().await; result
}
struct Work { request: SignedRequest, answer: oneshot::Sender<io::Result<(Reply, Option<crate::screen::core::capture_image::CapturedImage>)>> }
struct BrokerWorker { commands: Option<mpsc::Sender<Work>>, ready: Option<oneshot::Receiver<io::Result<(Challenge, Arc<crate::screen::core::image_admission::ImageAdmission>)>>>, worker: Option<JoinHandle<()>>, images: Option<Arc<crate::screen::core::image_admission::ImageAdmission>> }
impl BrokerWorker {
    fn start(peer: String, user: String) -> Self {
        let (commands, mut input) = mpsc::channel::<Work>(1);
        let (ready, initial) = oneshot::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let mut client = match GatewayClient::connect(&peer, &user) { Ok(client) => client, Err(error) => { let _ = ready.send(Err(error)); return; } };
            if ready.send(Ok((client.challenge().clone(), client.images()))).is_err() { return; }
            while let Some(work) = input.blocking_recv() {
                // Abandoned queued work must not become a new privileged IPC
                // request. An operation already handed to the broker is in flight.
                if input.is_closed() || work.answer.is_closed() { break; }
                let answer = client.send(work.request); let end = answer.is_err();
                if work.answer.send(answer).is_err() || end { break; }
            }
            // Dropping the only IPC client ends capture/input at the independent broker.
        });
        Self { commands: Some(commands), ready: Some(initial), worker: Some(worker), images: None }
    }
    async fn ready(&mut self) -> io::Result<Challenge> {
        let (challenge, images) = self.ready.take().ok_or_else(|| denied("broker startup challenge was already consumed"))?.await.map_err(io::Error::other)??;
        self.images = Some(images); Ok(challenge)
    }
    async fn ask(&mut self, request: SignedRequest) -> io::Result<(Reply, Option<crate::screen::core::capture_image::CapturedImage>)> {
        let (answer, received) = oneshot::channel();
        self.commands.as_ref().ok_or_else(|| denied("local broker bridge is closed"))?.send(Work { request, answer }).await.map_err(|_| io::Error::other("local broker bridge stopped"))?;
        received.await.map_err(io::Error::other)?
    }
    fn images(&self) -> io::Result<Arc<crate::screen::core::image_admission::ImageAdmission>> {self.images.clone().ok_or_else(|| denied("broker image admission is unavailable"))}
    async fn finish(mut self) { self.commands.take(); if let Some(worker) = self.worker.take() { let _ = worker.await; } }
}
impl Drop for BrokerWorker { fn drop(&mut self) { self.commands.take(); } }
