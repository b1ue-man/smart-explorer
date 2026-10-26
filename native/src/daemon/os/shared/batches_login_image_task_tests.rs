//! Real IPC reader/viewer and delayed writer/registry fixtures for remote CI.
use super::{registry, session};
use crate::{
    daemon::{
        ipc_protocol::{
            self,
            login::{
                self as wire, LoginCommand, LoginFrame, LoginIpcReply, LoginRelayCommand,
                LoginRelayReply, LoginSessionView,
            },
            IpcResponse,
        },
        LoginViewer,
    },
    login::{
        Challenge, DesktopKind, GatewayAccess, GatewayReply, GatewayTarget, LoginCancellation,
        LoginRights, SessionBinding, ViewerSession,
    },
    screen::core::{
        capture_admission::{CaptureGeneration, SourceKey},
        capture_image::{CaptureImageBuilder, CapturedImage},
        frame_budget::FrameLimits,
        frame_memory::{CapacityCeiling, MemoryPool},
        image_admission::{ImageAdmission, ImageOwnerLimits, ImageRole, LocalImagePolicy},
        pixel_convert::FrameSize,
    },
};
use std::{
    io::{self, Write},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Arc, Condvar, Mutex},
    time::Duration,
};
fn admit(role: ImageRole, count: u64) -> Arc<ImageAdmission> {
    ImageAdmission::in_test_pool(
        Arc::new(MemoryPool::default()),
        LocalImagePolicy {
            facts: None,
            ceiling: CapacityCeiling::bounded(512 * 1024 * 1024),
        },
        ImageOwnerLimits::new(8)
            .unwrap()
            .with_role(role, 4, count)
            .unwrap(),
    )
}
fn target() -> GatewayTarget {
    GatewayTarget {
        format: 1,
        machine_id: "fixture-machine".into(),
        public_key: iroh::SecretKey::from_bytes(&[7; 32]).public().to_string(),
        direct_addresses: Vec::new(),
        relay_urls: Vec::new(),
        use_default_relays: true,
        allow_plaintext_relays: false,
    }
}
fn view() -> LoginSessionView {
    let target = target();
    LoginSessionView {
        session: "bc".repeat(32),
        target: target.clone(),
        target_user: "fixture-user".into(),
        state: "challenge".into(),
        route: None,
        challenge: Some(Challenge {
            format: 1,
            machine_id: target.machine_id.clone(),
            machine_public_key: target.public_key.clone(),
            boot_generation: "fixture-boot".into(),
            binding: SessionBinding {
                session_id: "1".into(),
                user_id: Some("fixture-user".into()),
                session_generation: "fixture-session".into(),
                desktop_generation: "fixture-desktop".into(),
                desktop: DesktopKind::Locked,
            },
            policy_revision: 1,
            nonce: "de".repeat(32),
            sequence: 1,
            expires_at: u64::MAX,
        }),
        access: Some(GatewayAccess {
            peer_id: "fixture-peer".into(),
            public_key: target.public_key,
            target_user: "fixture-user".into(),
            policy_revision: 1,
            not_before: 0,
            expires_at: u64::MAX,
            rights: LoginRights {
                screen_view: true,
                screen_control: true,
                login: true,
                before_login: false,
                locked: true,
                uac: false,
                sas: false,
            },
        }),
    }
}
fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    (client, server)
}
fn image(
    admission: &Arc<ImageAdmission>,
    role: ImageRole,
    size: FrameSize,
    blue: u8,
) -> CapturedImage {
    let mut image = CaptureImageBuilder::new_local(
        size,
        FrameLimits::LEGACY,
        admission,
        role,
        SourceKey::new("fixture", "login:ipc").unwrap(),
        CaptureGeneration::new(1),
    )
    .unwrap();
    for pixel in image.pixels_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&[blue, 2, 3, 255]);
    }
    image.freeze()
}
fn frame(
    mut view: LoginSessionView,
    sequence: u64,
    image: Option<CapturedImage>,
    state: &str,
) -> LoginFrame {
    view.state = state.into();
    let mut reply = crate::login::Reply::state(state);
    reply.challenge = view.challenge.clone();
    if let Some(image) = &image {
        reply.width = image.size().width;
        reply.height = image.size().height;
        reply.pixel_bytes = image.image().data().len();
    }
    LoginFrame {
        reply: LoginRelayReply {
            sequence,
            view,
            packet: Some(GatewayReply {
                request_sequence: sequence,
                reply,
                access: None,
            }),
        },
        image,
    }
}
pub(crate) struct LoginImageFixture {
    pub session: Arc<ViewerSession>,
    pub admission: Arc<ImageAdmission>,
    pool: Arc<MemoryPool>,
    policy: LocalImagePolicy,
    captures: mpsc::Receiver<u64>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl LoginImageFixture {
    /// Leaves the real frame queued for LoginImage::show/take_frame.
    pub(crate) fn wait_frame(&self, wait: Duration) -> bool {
        self.session.visible();
        self.session.wait_fixture_ready(wait)
    }
    pub(crate) fn next_capture(&self, wait: Duration) -> Result<u64, mpsc::RecvTimeoutError> {
        self.captures.recv_timeout(wait)
    }
    pub(crate) fn image_admission(&self, limits: ImageOwnerLimits) -> Arc<ImageAdmission> {
        ImageAdmission::in_test_pool(self.pool.clone(), self.policy, limits)
    }
    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.session.close();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("login fixture peer panicked"))?;
        }
        Ok(())
    }
}
impl Drop for LoginImageFixture {
    fn drop(&mut self) {
        self.session.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// Two allowed fixed-size frames, then unchanged responses. No UI memory hold
/// is injected: source, PNG wire/decoder and IPC viewer share one real pool.
pub(crate) fn login_image_fixture(size: FrameSize) -> io::Result<LoginImageFixture> {
    size.check_frame()?;
    let (socket, mut server) = pair();
    let initial = view();
    let server_view = initial.clone();
    let pool = Arc::new(MemoryPool::default());
    let policy = LocalImagePolicy {
        facts: None,
        ceiling: CapacityCeiling::bounded(512 * 1024 * 1024),
    };
    let admission = ImageAdmission::in_test_pool(
        pool.clone(),
        policy,
        crate::login::os::shared::viewer_images::limits()?,
    );
    let sender = ImageAdmission::in_test_pool(
        pool.clone(),
        policy,
        crate::login::core::image_send::limits()?,
    );
    let receiver = crate::login::core::image_receive::ImageReceiver {
        admission: ImageAdmission::in_test_pool(
            pool.clone(),
            policy,
            crate::login::core::image_receive::ImageReceiver::limits()?,
        ),
        source: SourceKey::new(&initial.session, "login:network")?,
        cancelled: Arc::new(|| false),
        lifetime: None,
    };
    let (captures, events) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut view = server_view;
        let mut produced = 0;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        ipc_protocol::write_response(
            &mut server,
            &IpcResponse::Login(LoginIpcReply::StreamOpened {
                session: view.session.clone(),
            }),
        )
        .unwrap();
        wire::write_json(&mut server, wire::HELLO, &view).unwrap();
        while let Ok((tag, bytes)) = wire::read_frame(&mut server, wire::MAX_CONTROL) {
            if tag != wire::COMMAND {
                break;
            }
            let command: LoginRelayCommand = serde_json::from_slice(&bytes).unwrap();
            view.challenge.as_mut().unwrap().sequence = command.sequence + 1;
            let reply = match command.command {
                LoginCommand::Start => frame(view.clone(), command.sequence, None, "ready"),
                LoginCommand::Capture { frame_id } => {
                    produced += 1;
                    if produced <= 2 {
                        let decoded = runtime.block_on(async {
                            let source =
                                image(&sender, ImageRole::DecodedInput, size, produced as u8);
                            let packet = frame(
                                view.clone(),
                                command.sequence,
                                Some(source.clone()),
                                "pixels",
                            )
                            .reply
                            .packet
                            .unwrap();
                            let encoded = crate::login::core::gateway_wire::prepare_reply(
                                packet,
                                Some(source),
                                sender.clone(),
                            )
                            .await
                            .unwrap();
                            let (mut send, mut recv) = tokio::io::duplex(64);
                            let (sent, received) = tokio::join!(
                                crate::login::core::gateway_wire::send_prepared(
                                    &mut send, &encoded
                                ),
                                crate::login::core::gateway_wire::receive_reply(
                                    &mut recv, &receiver
                                )
                            );
                            sent.unwrap();
                            received.unwrap().1.unwrap()
                        });
                        captures.send(frame_id).unwrap();
                        frame(view.clone(), command.sequence, Some(decoded), "pixels")
                    } else {
                        frame(view.clone(), command.sequence, None, "unchanged")
                    }
                }
                LoginCommand::ReleaseInputs => {
                    frame(view.clone(), command.sequence, None, "inputs_released")
                }
                _ => break,
            };
            if wire::write_reply(&mut server, &reply).is_err() {
                break;
            }
        }
    });
    let viewer = LoginViewer::from_fixture_stream(
        socket,
        initial.target,
        initial.target_user,
        admission.clone(),
    )
    .map_err(io::Error::other)?;
    let session = ViewerSession::from_fixture_viewer(viewer, admission.clone())?;
    Ok(LoginImageFixture {
        session,
        admission,
        pool,
        policy,
        captures: events,
        worker: Some(worker),
    })
}
#[test]
fn batches_task_login_image_productive_ipc_viewer_keeps_two_real_raw_slots() {
    let fixture = login_image_fixture(FrameSize::new(1, 1)).unwrap();
    let session = fixture.session.clone();
    let images = fixture.admission.clone();
    let first = session.wait_fixture_frame(Duration::from_secs(2)).unwrap();
    assert_eq!(fixture.next_capture(Duration::from_secs(1)).unwrap(), 1);
    session.visible();
    let second = session.wait_fixture_frame(Duration::from_secs(2)).unwrap();
    assert_eq!(fixture.next_capture(Duration::from_secs(1)).unwrap(), 2);
    assert_eq!(first.image.image().data(), &[1, 2, 3, 255]);
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 2);
    assert!(
        fixture.next_capture(Duration::from_millis(250)).is_err(),
        "externally held frames stop a third capture before allocation"
    );
    session.visible();
    session.release();
    assert!(
        !matches!(session.snapshot().state.as_str(), "ended" | "stopping"),
        "focus release without held input preserves the session"
    );
    fixture.finish().unwrap();
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 2);
    drop(first);
    drop(second);
    assert_eq!(images.live_bytes().unwrap(), 0);
}
struct Delayed {
    gate: Arc<(Mutex<bool>, Condvar)>,
    started: Option<mpsc::Sender<()>>,
}
struct ReleaseGate(Arc<(Mutex<bool>, Condvar)>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        let mut released = self.0 .0.lock().unwrap();
        *released = true;
        self.0 .1.notify_all();
    }
}
impl Write for Delayed {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
            let lock = self.gate.0.lock().unwrap();
            drop(self.gate.1.wait_while(lock, |release| !*release).unwrap());
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn batches_task_login_image_writer_completion_keeps_real_registry_owner() {
    let (_socket, server) = pair();
    let view = view();
    let id = view.session.clone();
    let owner =
        registry::register(view.clone(), Arc::new(LoginCancellation::default()), server).unwrap();
    let admission = admit(ImageRole::DecodedInput, 1);
    let image = image(&admission, ImageRole::DecodedInput, FrameSize::new(1, 1), 1);
    image.retain_lifetime(Arc::new(owner.clone())).unwrap();
    let frame = frame(view, 1, Some(image), "pixels");
    drop(owner);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let thread_gate = gate.clone();
    let _release_on_failure = ReleaseGate(gate.clone());
    let (started, ready) = mpsc::channel();
    let (complete, mut written) = tokio::sync::oneshot::channel();
    let worker = std::thread::spawn(move || {
        session::write_owned(
            &mut Delayed {
                gate: thread_gate,
                started: Some(started),
            },
            frame,
            complete,
        )
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(written.try_recv().is_err());
    assert!(registry::session(&id).unwrap().is_some());
    assert_eq!(admission.live_bytes().unwrap(), 4);
    {
        let mut released = gate.0.lock().unwrap();
        *released = true;
        gate.1.notify_all();
    }
    worker.join().unwrap().unwrap();
    written.blocking_recv().unwrap().unwrap();
    assert!(registry::session(&id).unwrap().is_none());
    assert_eq!(admission.live_bytes().unwrap(), 0);
}
