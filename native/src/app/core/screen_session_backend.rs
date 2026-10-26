//! Production transport and renderer admission.
use super::screen_session::Shared;
use crate::screen::core::{
    host_backend::ScreenDecoder, negotiate::CodecOffer, wire_binary::GopHeader,
    wire_control::ViewerFrame,
};
use crate::share::access_grant::CapabilityDecision;
use crate::{
    daemon::{StreamEvent, StreamRequest},
    share::PeerOpenTarget,
};
use eframe::egui;
use std::{io, sync::Arc};
/// Wakes the GUI thread (production: `Context::request_repaint`).
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// A relayed session as plain functions, so the daemon's `ScreenStream`
/// need not be named and a test can stand in for the host.
pub(crate) struct StreamLink {
    pub memory: Option<Arc<crate::screen::core::frame_memory::MemoryLease>>,
    /// The next event; blocks until one arrives.
    pub next: Box<dyn FnMut() -> io::Result<StreamEvent> + Send>,
    /// Sends a control frame to the host.
    pub send: Arc<dyn Fn(&ViewerFrame) -> io::Result<()> + Send + Sync>,
    /// Ends the session.
    pub close: Arc<dyn Fn() -> io::Result<()> + Send + Sync>,
}

pub(crate) type Opener =
    Arc<dyn Fn(PeerOpenTarget, StreamRequest, &Shared) -> Result<StreamLink, String> + Send + Sync>;
pub(crate) type DecoderOpener =
    Arc<dyn Fn(&GopHeader) -> io::Result<Box<dyn ScreenDecoder>> + Send + Sync>;

/// What a probe of a device tells the group view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProbeInfo {
    /// The device knows the screen protocol of this version.
    pub supported: bool,
    pub view: CapabilityDecision,
}

pub(crate) type Prober = Arc<dyn Fn(PeerOpenTarget) -> Result<ProbeInfo, String> + Send + Sync>;

/// Everything a session needs from outside.
#[derive(Clone)]
pub(crate) struct Backend {
    pub open: Opener,
    pub decoder: DecoderOpener,
    pub probe: Prober,
    pub wake: Wake,
    /// The decoders this process offers the host.
    pub offers: Vec<CodecOffer>,
}

impl Backend {
    /// The daemon's relay and the codecs of this process.
    pub(crate) fn production(ctx: &egui::Context) -> Self {
        let registry = Arc::new(crate::screen::os::codec_inventory::registry());
        let offers = registry.decoder_offers();
        let renderer = ctx.clone();
        let open: Opener = Arc::new(
            move |target: PeerOpenTarget,
                  mut request: StreamRequest,
                  shared: &Shared|
                  -> Result<StreamLink, String> {
                if let Some(owner) = super::screen_render_owner::get(&renderer) {
                    owner.attach(&shared.retirement);
                }
                let (caps, memory) = super::screen_render_caps::admit_wait(&renderer, shared)
                    .map_err(|error| error.to_string())?;
                request.frame_capabilities = Some(caps);
                let mut stream = crate::daemon::screen_stream(target, request)?;
                let control = stream.control();
                let (sender, closer) = (control.clone(), control);
                Ok(StreamLink {
                    memory: Some(Arc::new(memory)),
                    next: Box::new(move || stream.next_event()),
                    send: Arc::new(move |frame: &ViewerFrame| sender.send(frame)),
                    close: Arc::new(move || closer.close()),
                })
            },
        );
        let renderer = ctx.clone();
        let decoder: DecoderOpener = Arc::new(move |header: &GopHeader| {
            super::screen_render_caps::check(&renderer, header.size)?;
            registry.open_decoder(header)
        });
        let probe: Prober = Arc::new(|target| {
            crate::daemon::screen_probe(target).map(|report| ProbeInfo {
                supported: report.supported,
                view: report.view,
            })
        });
        let ctx = ctx.clone();
        Backend {
            open,
            decoder,
            probe,
            wake: Arc::new(move || ctx.request_repaint()),
            offers,
        }
    }
}
