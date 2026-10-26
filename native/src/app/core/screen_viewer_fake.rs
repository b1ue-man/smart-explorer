//! Test support of the desktop viewer (SCR-M7): a stand-in for the host that
//! hands out the events of each connection and records what the viewer sends,
//! and the profiles of a device.
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::screen_pool::{preview_request, ViewOpen};
use super::screen_prefs::ViewerPrefs;
use super::screen_session::{
    Backend, DecoderOpener, Opener, ProbeInfo, Prober, StreamLink, ViewerSession,
};
use super::screen_target::ScreenTarget;
use crate::daemon::{StreamEvent, StreamRequest};
use crate::screen::core::host_backend::{ScreenDecoder, TilesScreenDecoder};
use crate::screen::core::input_model::{
    InputCapabilities, SystemAction, KEY_SUPER_LEFT, KEY_VOLUME_UP,
};
use crate::screen::core::negotiate::CodecOffer;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::GopHeader;
use crate::screen::core::wire_control::{StreamMode, ViewerFrame};
use crate::screen::core::wire_hello::{
    ApprovalKind, HostPlatform, ScreenAuthorization, ScreenHelloOk,
};
use crate::share::access_grant::CapabilityDecision;
use crate::share::{DirectAccessState, DirectContact, PeerOpenTarget, ShareProfiles, ShareStatus};

pub(super) const DISPLAY: FrameSize = FrameSize::new(1280, 720);

pub(super) fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The host: counts connections and probes, keeps the frames the viewer
/// sent and lets the test push the events of each connection.
#[derive(Clone)]
pub(super) struct Fake {
    pub(super) view: CapabilityDecision,
    pub(super) inner: Arc<Inner>,
}

#[derive(Default)]
pub(super) struct Inner {
    pub(super) opens: AtomicUsize,
    pub(super) probes: AtomicUsize,
    pub(super) requests: Mutex<Vec<StreamRequest>>,
    pub(super) frames: Mutex<Vec<ViewerFrame>>,
    pub(super) events: Mutex<Vec<mpsc::Sender<io::Result<StreamEvent>>>>,
    pub(super) closed: Mutex<Vec<usize>>,
}

pub(super) fn hello(view: CapabilityDecision) -> ScreenHelloOk {
    ScreenHelloOk {
        family_version: None,
        icon_version: None,
        frame_contract: None,
        authorization: ScreenAuthorization {
            view,
            control: CapabilityDecision::Allow,
        },
        platform: HostPlatform::Linux,
        encoders: vec![CodecOffer::tiles()],
        input: InputCapabilities {
            pointer: true,
            keyboard: true,
            text: true,
            touch: false,
            actions: vec![SystemAction::Back],
            toolbar_keys: vec![KEY_SUPER_LEFT, KEY_VOLUME_UP],
        },
        monitors: Vec::new(),
        limits: Vec::new(),
        approval: ApprovalKind::NotRequired,
    }
}

impl Fake {
    pub(super) fn new(view: CapabilityDecision) -> Self {
        Self {
            view,
            inner: Arc::default(),
        }
    }

    pub(super) fn backend(&self) -> Backend {
        let inner = self.inner.clone();
        let open: Opener = Arc::new(
            move |_peer: PeerOpenTarget,
                  request: StreamRequest,
                  _shared: &super::screen_session::Shared| {
                let (sender, receiver) = mpsc::channel();
                inner.requests.lock().unwrap().push(request);
                // The test sees the connection once its events can be sent.
                let index = {
                    let mut senders = inner.events.lock().unwrap();
                    senders.push(sender);
                    senders.len() - 1
                };
                inner.opens.fetch_add(1, Ordering::SeqCst);
                let (frames, closer) = (inner.clone(), inner.clone());
                Ok::<StreamLink, String>(StreamLink {
                    memory: None,
                    next: Box::new(move || {
                        receiver
                            .recv()
                            .unwrap_or_else(|_| Err(io::Error::other("closed")))
                    }),
                    send: Arc::new(move |frame: &ViewerFrame| {
                        frames.frames.lock().unwrap().push(frame.clone());
                        Ok::<(), io::Error>(())
                    }),
                    close: Arc::new(move || {
                        closer.closed.lock().unwrap().push(index);
                        Ok::<(), io::Error>(())
                    }),
                })
            },
        );
        let decoder: DecoderOpener = Arc::new(|header: &GopHeader| {
            Ok::<Box<dyn ScreenDecoder>, io::Error>(Box::new(TilesScreenDecoder::new(header.size)?))
        });
        let (inner, view) = (self.inner.clone(), self.view);
        let probe: Prober = Arc::new(move |_target: PeerOpenTarget| {
            inner.probes.fetch_add(1, Ordering::SeqCst);
            Ok::<ProbeInfo, String>(ProbeInfo {
                supported: true,
                view,
            })
        });
        Backend {
            open,
            decoder,
            probe,
            wake: Arc::new(|| {}),
            offers: vec![CodecOffer::tiles()],
        }
    }

    pub(super) fn opens(&self) -> usize {
        self.inner.opens.load(Ordering::SeqCst)
    }

    pub(super) fn accept(&self, index: usize) {
        let event = StreamEvent::Accepted {
            hello: hello(self.view),
            path: "direct".into(),
        };
        let senders = self.inner.events.lock().unwrap();
        senders[index].send(Ok(event)).unwrap();
    }

    pub(super) fn frames(&self) -> Vec<ViewerFrame> {
        self.inner.frames.lock().unwrap().clone()
    }

    pub(super) fn closed(&self, index: usize) -> bool {
        self.inner.closed.lock().unwrap().contains(&index)
    }
}

pub(super) fn target() -> ScreenTarget {
    ScreenTarget::direct("c1", Some("device-c1"))
}

pub(super) fn open_preview(fake: &Fake, backend: &Backend) -> ViewerSession {
    let request = preview_request(&backend.offers, 320, 1000);
    let session = ViewerSession::open(backend, target(), request, false, false, 0);
    wait_for("the connection", || fake.opens() >= 1);
    session
}

pub(super) fn open() -> ViewOpen {
    ViewOpen {
        display: DISPLAY,
        prefs: ViewerPrefs::default(),
        prefer_yuv: false,
    }
}

pub(super) fn set_modes(fake: &Fake) -> Vec<StreamMode> {
    fake.frames()
        .into_iter()
        .filter_map(|frame| match frame {
            ViewerFrame::SetMode(set) => Some(set.mode),
            _ => None,
        })
        .collect()
}

fn contact(status: ShareStatus, device_id: Option<&str>) -> DirectContact {
    DirectContact {
        id: "c1".into(),
        display_name: "Laptop".into(),
        lookup_id: "lookup".into(),
        expected_fingerprint: "fp".into(),
        expected_node_id: "node".into(),
        remote_device_id: device_id.map(str::to_string),
        remote_public_key: Some("key".into()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

pub(super) fn profiles(status: ShareStatus, device_id: Option<&str>) -> ShareProfiles {
    ShareProfiles {
        direct_contacts: vec![contact(status, device_id)],
        ..ShareProfiles::default()
    }
}
