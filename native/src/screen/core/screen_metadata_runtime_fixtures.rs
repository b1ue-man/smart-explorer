//! Isolated fixtures for the screen metadata remote task suite.
use super::{
    app_icon::{AuthorizedIconEntry, IconReply, IconRequest, IconTarget},
    authorize::ScreenAuthority,
    family_protocol::{FamilyListEntry, FamilyListing, FAMILY_PROTOCOL},
    frame_memory::MemoryFacts,
    host_backend::{
        PreparedSource, ScreenEncoder, ScreenHostBackend, ScreenSource, SessionIndicator,
        SourceRefusal,
    },
    host_server::HostContext,
    input_model::{InputCapabilities, InputEvent},
    negotiate::{CodecChoice, CodecOffer},
    pixel_convert::FrameSize,
    registry::{NewSession, ScreenRegistry, SessionHandle},
    session_state::SessionState,
    window_list::{AppEntry, ListSource, MonitorInfo, SourceId, WindowEntry, WindowList},
    wire_control::ControlBlock,
    wire_hello::{HostPlatform, ScreenClientHello, SessionMode},
};
use crate::share::{
    access_approval::{ApprovalService, NoConsent},
    access_grant::{Capability, CapabilityDecision, CapabilityState},
    relation_proof::ProvenPrincipal,
};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Condvar, Mutex,
};
use std::{cell::Cell, future::Future, io, time::Duration};
use tokio::sync::Notify;

pub(crate) const DEADLINE: Duration = Duration::from_secs(5);

thread_local! {
    /// Set while this thread runs the callback of `ScreenAuthority::admit`.
    static ADMITTING: Cell<bool> = const { Cell::new(false) };
}
fn allows(state: (bool, bool), capability: Capability) -> bool {
    if capability == Capability::ScreenControl {
        state.1
    } else {
        state.0
    }
}

pub(crate) struct Rights(pub Mutex<(bool, bool)>);
impl ScreenAuthority for Rights {
    fn capability(
        &self,
        _: &ProvenPrincipal,
        capability: Capability,
        _: i64,
    ) -> Option<CapabilityState> {
        let allowed = allows(*self.0.lock().unwrap(), capability);
        Some(CapabilityState {
            decision: if allowed {
                CapabilityDecision::Allow
            } else {
                CapabilityDecision::Off
            },
            policy_revision: 1,
            changed_at: 0,
        })
    }
    fn admit(
        &self,
        _: &ProvenPrincipal,
        needed: &[Capability],
        _: i64,
        admit: &mut dyn FnMut(),
    ) -> bool {
        let state = self.0.lock().unwrap();
        let allowed = needed.iter().all(|capability| allows(*state, *capability));
        if allowed {
            ADMITTING.with(|inside| inside.set(true));
            admit();
            ADMITTING.with(|inside| inside.set(false));
        }
        allowed
    }
    fn stored_names(&self, _: &ProvenPrincipal) -> (Option<String>, Option<String>) {
        (Some("Metadata fixture".into()), None)
    }
}

pub(crate) struct Block {
    released: Mutex<bool>,
    changed: Condvar,
    pub entered: Notify,
}
impl Block {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            released: Mutex::new(false),
            changed: Condvar::new(),
            entered: Notify::new(),
        })
    }
    fn wait(&self) {
        self.entered.notify_one();
        let released = self.released.lock().unwrap();
        let (released, _) = self
            .changed
            .wait_timeout_while(released, DEADLINE, |released| !*released)
            .unwrap();
        assert!(*released, "fixture blocking query was not released");
    }
    pub(crate) fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
pub(crate) struct Release(pub Arc<Block>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

pub(crate) struct Backend {
    pub reply: IconReply,
    pub calls: AtomicUsize,
    pub opened: AtomicUsize,
    pub icon_failure: AtomicBool,
    pub block: Mutex<Option<(&'static str, Arc<Block>)>>,
    pub panic_at: Mutex<Option<&'static str>>,
}
impl Backend {
    fn touch(&self, name: &'static str) {
        // The original fault: platform work inside the shared authorization.
        assert!(
            !ADMITTING.with(Cell::get),
            "platform query inside share authorization: {name}"
        );
        // Same runtime nesting contract as zbus::blocking with its Tokio feature.
        let inner = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        inner.block_on(async {});
        self.calls.fetch_add(1, Ordering::SeqCst);
        let block = self
            .block
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(wanted, _)| *wanted == name)
            .map(|(_, block)| block.clone());
        if let Some(block) = block {
            block.wait();
        }
        let panic = {
            let mut slot = self.panic_at.lock().unwrap();
            if *slot == Some(name) {
                slot.take();
                true
            } else {
                false
            }
        };
        assert!(!panic, "isolated fixture backend panic: {name}");
    }
    pub(crate) fn list(&self) -> WindowList {
        WindowList {
            windows: vec![WindowEntry {
                id: "fixture-window".into(),
                title: "Fixture document".into(),
                app: "Fixture".into(),
                app_id: Some(self.reply.icon.app_id.clone()),
                icon: Some(self.reply.icon.clone()),
                pid: Some(42),
                monitor: Some("fixture-monitor".into()),
                minimized: false,
                focused: true,
            }],
            apps: vec![AppEntry {
                app_id: self.reply.icon.app_id.clone(),
                name: "Fixture".into(),
                icon: Some(self.reply.icon.clone()),
                pid: Some(42),
            }],
            list_source: ListSource::Ewmh,
            limits: vec!["fixture-list-limit".into()],
        }
    }
    pub(crate) fn icon_request(&self, window: bool) -> IconRequest {
        IconRequest {
            target: if window {
                IconTarget::Window {
                    id: "fixture-window".into(),
                }
            } else {
                IconTarget::App {
                    app_id: self.reply.icon.app_id.clone(),
                }
            },
            icon: self.reply.icon.clone(),
        }
    }
}
impl ScreenHostBackend for Backend {
    fn platform(&self) -> HostPlatform {
        HostPlatform::Linux
    }
    fn graphical_session(&self) -> bool {
        true
    }
    fn memory_facts(&self) -> Option<MemoryFacts> {
        Some(MemoryFacts {
            available_physical: 1 << 40,
            available_commit: 1 << 40,
            available_address_space: 1 << 40,
        })
    }
    fn monitors(&self) -> Vec<MonitorInfo> {
        self.touch("monitors");
        vec![MonitorInfo {
            id: "fixture-monitor".into(),
            name: "Fixture monitor".into(),
            x: 0,
            y: 0,
            width: 64,
            height: 64,
            primary: true,
        }]
    }
    fn encoders(&self) -> Vec<CodecOffer> {
        self.touch("encoders");
        vec![CodecOffer::tiles()]
    }
    fn input(&self) -> InputCapabilities {
        InputCapabilities::default()
    }
    fn limits(&self) -> Vec<String> {
        self.touch("limits");
        vec!["fixture-host-limit".into()]
    }
    fn windows(&self) -> io::Result<WindowList> {
        self.touch("windows");
        Ok(self.list())
    }
    fn family_windows(&self, owner: &str) -> io::Result<FamilyListing> {
        self.touch("families");
        assert!(owner.contains("metadata-runtime-fixture"));
        let session = "1234567890abcdef1234567890abcdef";
        Ok(FamilyListing {
            version: FAMILY_PROTOCOL,
            entries: vec![FamilyListEntry {
                launch_request: Some("fixture-launch-request".into()),
                source: SourceId::Window(format!("@family.{session}")),
                identity: crate::remote_window::WindowIdentity {
                    session: session.into(),
                    generation: 1,
                },
                window: self.list().windows.remove(0),
            }],
            limitations: vec!["fixture-family-limit".into()],
        })
    }
    fn icon(&self, _: AuthorizedIconEntry<'_>) -> io::Result<Option<IconReply>> {
        self.touch("icon");
        if self.icon_failure.load(Ordering::SeqCst) {
            return Err(io::Error::other("fixture icon unavailable"));
        }
        Ok(Some(self.reply.clone()))
    }
    fn prepare_session_source(
        &self,
        owner: &str,
        session: &str,
        source: &SourceId,
        wanted: Option<FrameSize>,
        control: bool,
        admission: Arc<super::capture_admission::SessionAdmission>,
    ) -> Result<Box<dyn PreparedSource>, SourceRefusal> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        crate::screen::os::synthetic_backend::SyntheticBackend
            .prepare_session_source(owner, session, source, wanted, control, admission)
    }
    fn open_source(
        &self,
        _: &SourceId,
        _: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        Err(SourceRefusal::new("fixture requires prepared capture"))
    }
    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        crate::screen::os::synthetic_backend::SyntheticBackend.open_encoder(choice, size)
    }
    fn open_encoder_for_stage_admitted(
        &self,
        request: &super::host_stage::StageRequest,
        grant: super::capture_admission::CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        crate::screen::os::synthetic_backend::SyntheticBackend
            .open_encoder_for_stage_admitted(request, grant)
    }
    fn inject(&self, _: &[InputEvent]) -> Result<(), ControlBlock> {
        Ok(())
    }
}

pub(crate) struct Indicator;
impl SessionIndicator for Indicator {
    fn available(&self) -> bool {
        true
    }
    fn show(&self) -> Result<(), String> {
        Ok(())
    }
}
pub(crate) struct Rig {
    pub context: HostContext,
    pub rights: Arc<Rights>,
    pub backend: Arc<Backend>,
    pub principal: ProvenPrincipal,
}
pub(crate) fn rig() -> Rig {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[20, 40, 60, 255])
            .unwrap();
    }
    let rights = Arc::new(Rights(Mutex::new((true, true))));
    let backend = Arc::new(Backend {
        reply: IconReply::new("fixture.desktop", &bytes).unwrap(),
        calls: AtomicUsize::new(0),
        opened: AtomicUsize::new(0),
        icon_failure: AtomicBool::new(false),
        block: Mutex::new(None),
        panic_at: Mutex::new(None),
    });
    let context = HostContext {
        backend: backend.clone(),
        registry: Arc::new(ScreenRegistry::default()),
        authority: rights.clone(),
        broker: Arc::new(ApprovalService::new(Arc::new(NoConsent)).broker()),
        indicator: Arc::new(Indicator),
        audit: |_| {},
        inject_resets: Arc::new(std::sync::atomic::AtomicU32::new(0)),
    };
    let principal = ProvenPrincipal {
        relation_kind: "direct".into(),
        relation_id: "fixture-relation".into(),
        device_id: "metadata-runtime-fixture".into(),
        device_name: "Metadata fixture".into(),
        public_key: "fixture-key".into(),
        fingerprint: "fixture-fingerprint".into(),
        node_id: "fixture-node".into(),
    };
    Rig {
        context,
        rights,
        backend,
        principal,
    }
}
/// Changes the rights; an authorization lock held across platform work fails here.
pub(crate) async fn set_rights(rig: &Rig, rights: (bool, bool)) {
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while tokio::time::Instant::now() < deadline {
        if let Ok(mut state) = rig.rights.0.try_lock() {
            *state = rights;
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("share authorization stayed locked behind platform work");
}
pub(crate) fn handle(rig: &Rig, mode: SessionMode) -> SessionHandle {
    rig.context
        .registry
        .register(NewSession {
            principal: rig.principal.clone(),
            device_name: "Metadata fixture".into(),
            mode,
            source: "synthetic".into(),
            path: "direct".into(),
            started_at: 0,
            state: SessionState::Starting,
            screen_content: false,
        })
        .unwrap()
}
pub(crate) fn hello(mode: SessionMode, metadata: bool) -> ScreenClientHello {
    ScreenClientHello {
        protocol_version: super::wire::SCREEN_PROTOCOL_VERSION,
        relation_kind: "direct".into(),
        relation_id: "fixture-relation".into(),
        device_id: "metadata-runtime-fixture".into(),
        device_name: "Metadata fixture".into(),
        public_key: "fixture-key".into(),
        fingerprint: "fixture-fingerprint".into(),
        node_id: "fixture-node".into(),
        client_nonce: "fixture-nonce".into(),
        proof: "fixture-proof".into(),
        mode,
        source: Some(SourceId::Synthetic),
        decoders: vec![CodecOffer::tiles()],
        display_size: Some(FrameSize::new(64, 64)),
        max_fps: 30,
        frame_capabilities: None,
        icon_version: Some(super::app_icon::ICON_VERSION),
        family_version: Some(FAMILY_PROTOCOL),
        metadata_only: metadata,
    }
}

pub(crate) fn run<F: Future>(multi: bool, work: F) -> F::Output {
    let mut builder = if multi {
        tokio::runtime::Builder::new_multi_thread()
    } else {
        tokio::runtime::Builder::new_current_thread()
    };
    if multi {
        builder.worker_threads(2);
    }
    builder.enable_all().build().unwrap().block_on(work)
}
pub(crate) async fn bounded<F: Future>(work: F) -> F::Output {
    tokio::time::timeout(DEADLINE, work)
        .await
        .expect("metadata runtime fixture deadline")
}

pub(crate) struct Link {
    pub left: iroh::Endpoint,
    pub right: iroh::Endpoint,
    pub host: Connection,
    pub viewer: Connection,
}
/// Host send/receive, then viewer send/receive of one stream pair.
pub(crate) type Streams = (SendStream, RecvStream, SendStream, RecvStream);
pub(crate) async fn link() -> Link {
    async fn endpoint() -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup()
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .alpns(vec![b"metadata-runtime-fixture/1".to_vec()])
            .bind()
            .await
            .unwrap()
    }
    let left = endpoint().await;
    let right = endpoint().await;
    let address = iroh::EndpointAddr::from_parts(
        right.id(),
        right
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );
    let (host, viewer) = tokio::join!(
        left.connect(address, b"metadata-runtime-fixture/1"),
        async { right.accept().await.unwrap().await }
    );
    Link {
        left,
        right,
        host: host.unwrap(),
        viewer: viewer.unwrap(),
    }
}
impl Link {
    pub(crate) async fn streams(&self) -> Streams {
        use tokio::io::AsyncReadExt;
        let (mut host_send, host_recv) = self.host.open_bi().await.unwrap();
        host_send.write_all(&[0]).await.unwrap();
        let (viewer_send, mut viewer_recv) = self.viewer.accept_bi().await.unwrap();
        assert_eq!(viewer_recv.read_u8().await.unwrap(), 0);
        (host_send, host_recv, viewer_send, viewer_recv)
    }
    pub(crate) async fn close(self) {
        self.host.close(0u32.into(), b"fixture done");
        self.viewer.close(0u32.into(), b"fixture done");
        self.left.close().await;
        self.right.close().await;
    }
}
