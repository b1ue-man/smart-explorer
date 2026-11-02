//! The login image fixture: a real IPC viewer fed by a fixture gateway peer
//! that sends two allowed frames through the real PNG wire (remote CI tests of
//! the daemon and the GUI; reachable there through `test-support`).
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
        Challenge, DesktopKind, GatewayAccess, GatewayReply, GatewayTarget, LoginRights,
        SessionBinding, ViewerSession,
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
    io,
    net::{TcpListener, TcpStream},
    sync::{mpsc, Arc},
    time::Duration,
};
pub(super) fn target() -> GatewayTarget {
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
pub(super) fn view() -> LoginSessionView {
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
pub(super) fn pair() -> (TcpStream, TcpStream) {
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
pub(super) fn image(
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
pub(super) fn frame(
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
pub struct LoginImageFixture {
    pub session: Arc<ViewerSession>,
    pub admission: Arc<ImageAdmission>,
    pool: Arc<MemoryPool>,
    policy: LocalImagePolicy,
    captures: mpsc::Receiver<u64>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl LoginImageFixture {
    /// Leaves the real frame queued for LoginImage::show/take_frame.
    pub fn wait_frame(&self, wait: Duration) -> bool {
        self.session.visible();
        self.session.wait_fixture_ready(wait)
    }
    pub fn next_capture(&self, wait: Duration) -> Result<u64, mpsc::RecvTimeoutError> {
        self.captures.recv_timeout(wait)
    }
    pub fn image_admission(&self, limits: ImageOwnerLimits) -> Arc<ImageAdmission> {
        ImageAdmission::in_test_pool(self.pool.clone(), self.policy, limits)
    }
    pub fn finish(mut self) -> io::Result<()> {
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
pub fn login_image_fixture(size: FrameSize) -> io::Result<LoginImageFixture> {
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
