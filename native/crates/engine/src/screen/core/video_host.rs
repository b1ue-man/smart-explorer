//! The host session's side of the picture carriers (`video_transport.rs`):
//! the viewer's `VIDEO_OFFER` opens the datagram carriers for the pipeline
//! (`VideoRoute::offer`) and, for `video_udp_v1`, starts the link task of
//! the direct UDP pair (`screen/os/shared/video_udp.rs`); its
//! `VIDEO_TCP_OFFER` starts the task of the direct TCP link
//! (`screen/os/shared/video_tcp_carrier.rs`). Both tasks hand the frames
//! for the viewer (`VIDEO_ANSWER`, `VIDEO_TCP_ANSWER`, `VIDEO_TCP_RESULT`)
//! to the session, which writes them with `VIDEO_TAIL` (`next_frame`).
//! Before the first of these sockets opens, the firewall is asked once per
//! session in the background (`ensure_firewall_in_background`).
//! `FRAME_LOSS` goes to the pipeline, which continues the chain or starts
//! a key frame; `VIDEO_REPORT`, the carriers and the host's counters fill
//! `STATS` (`report`).
use std::io;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::super::host_pipeline::PipelineActor;
use super::super::video_tcp_wire::VideoTcpResult;
use super::super::wire_control::{HostStats, ViewerFrame};
use super::super::wire_hello::{ScreenClientHello, SessionMode};
use super::video_route::VideoRoute;
use super::{stun_server, Carriers, VideoOffer, VIDEO_TAIL};

/// Frames for the viewer the link tasks may queue before the session
/// writes them.
const OUTGOING: usize = 8;

pub struct VideoHost {
    /// The session streams pictures at all (no probe, no metadata).
    pictures: bool,
    offered: bool,
    tcp_offered: bool,
    firewall_asked: bool,
    route: Option<Arc<VideoRoute>>,
    tasks: Vec<JoinHandle<()>>,
    outgoing: mpsc::Receiver<(u8, Vec<u8>)>,
    sender: mpsc::Sender<(u8, Vec<u8>)>,
}

impl Default for VideoHost {
    fn default() -> Self {
        Self::with_pictures(false)
    }
}

impl VideoHost {
    fn with_pictures(pictures: bool) -> Self {
        let (sender, outgoing) = mpsc::channel(OUTGOING);
        Self {
            pictures,
            offered: false,
            tcp_offered: false,
            firewall_asked: false,
            route: None,
            tasks: Vec::new(),
            outgoing,
            sender,
        }
    }

    /// The session `hello` opened.
    pub fn for_hello(hello: &ScreenClientHello) -> Self {
        Self::with_pictures(!hello.metadata_only && hello.mode != SessionMode::Probe)
    }

    /// `VIDEO_OFFER`, `VIDEO_TCP_OFFER`, `VIDEO_TCP_RESULT`, `FRAME_LOSS`
    /// and `VIDEO_REPORT`; another frame is refused.
    pub fn on_frame(&mut self, frame: ViewerFrame, pipeline: &mut PipelineActor) -> io::Result<()> {
        match frame {
            ViewerFrame::VideoOffer(offer) => self.offer(offer, pipeline),
            ViewerFrame::VideoTcpOffer(offer) => {
                // One link per session; a probe or metadata channel has none.
                if self.tcp_offered || !self.pictures {
                    return Ok(());
                }
                self.tcp_offered = true;
                let route = self.route(pipeline);
                self.ask_firewall();
                let task = crate::screen::os::video_tcp_carrier::spawn_host_tcp(
                    offer,
                    route,
                    self.sender.clone(),
                );
                self.tasks.push(task);
                Ok(())
            }
            ViewerFrame::VideoTcpResult(result) => {
                let route = self.route(pipeline);
                match result {
                    VideoTcpResult::Connected { remote, path } => route.note(&format!(
                        "the viewer reached the direct TCP link from {remote} ({})",
                        path.label()
                    )),
                    VideoTcpResult::Failed { reason } => {
                        let failure =
                            crate::screen::os::video_tcp_link::TcpLinkFailure::new(reason, None);
                        route.note(&format!("viewer: {}", failure.status_line()));
                        route.set_tcp_failure(Some(failure.status_line()));
                    }
                }
                Ok(())
            }
            ViewerFrame::FrameLoss(loss) => {
                if let Some(route) = &self.route {
                    route.note_loss(loss);
                    // Applied at once, or after the running picture.
                    pipeline.poll();
                }
                Ok(())
            }
            ViewerFrame::VideoReport(report) => {
                if let Some(route) = &self.route {
                    route.note_report(report);
                }
                Ok(())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a frame of the picture carriers",
            )),
        }
    }

    fn route(&mut self, pipeline: &PipelineActor) -> Arc<VideoRoute> {
        self.route
            .get_or_insert_with(|| pipeline.video_route())
            .clone()
    }

    /// The firewall admits the session's own sockets (UDP pair, TCP link):
    /// asked once per session, never waited for.
    fn ask_firewall(&mut self) {
        if !std::mem::replace(&mut self.firewall_asked, true) {
            crate::screen::os::video_udp::ensure_firewall_in_background("host");
        }
    }

    fn offer(&mut self, offer: VideoOffer, pipeline: &mut PipelineActor) -> io::Result<()> {
        offer.validate()?;
        // One offer per session; a probe or metadata channel takes none.
        if self.offered || !self.pictures {
            return Ok(());
        }
        self.offered = true;
        let ours = Carriers {
            datagrams: true,
            udp: true,
        };
        let carriers = Carriers::of(&offer.carriers).and(ours);
        let route = self.route(pipeline);
        route.note(&format!(
            "viewer reads {}",
            if carriers.any() {
                carriers.names().join(", ")
            } else {
                "no carrier of this host".to_string()
            }
        ));
        route.offer(carriers);
        if carriers.udp {
            self.ask_firewall();
            let link = crate::screen::os::video_udp::spawn_host_link(
                offer.candidates(),
                route,
                stun_server(),
                self.sender.clone(),
            );
            self.tasks.push(link);
        }
        Ok(())
    }

    /// The next frame for the viewer outside `HostFrame`: the last picture
    /// before a still screen (`VIDEO_TAIL`), then what the link tasks
    /// queued.
    pub fn next_frame(&mut self) -> Option<(u8, Vec<u8>)> {
        let tail = self
            .route
            .as_ref()
            .and_then(|route| route.take_tail(Instant::now()));
        match tail {
            Some(tail) => Some((VIDEO_TAIL, tail.encode())),
            None => self.outgoing.try_recv().ok(),
        }
    }

    /// The carrier fields of `STATS`.
    pub fn report(&self, stats: &mut HostStats) {
        if let (Some(route), Some(chain)) = (&self.route, stats.chain.as_mut()) {
            route.report_into(chain);
        }
    }
}

impl Drop for VideoHost {
    /// A session that ends takes its direct pair and link along.
    fn drop(&mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }
}
