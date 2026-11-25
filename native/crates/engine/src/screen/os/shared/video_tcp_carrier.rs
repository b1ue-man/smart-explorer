//! The direct TCP link as the carrier of the pictures under „TCP“
//! (`screen/core/video_transport.rs`; the link itself:
//! `video_tcp_link.rs`, its frames: `screen/core/video_tcp_wire.rs`).
//!
//! Host: on the viewer's `VIDEO_TCP_OFFER` a task gathers its candidates
//! (the Share server's TCP reflector from `share::address_services`), queues
//! `VIDEO_TCP_ANSWER` for the session, establishes the link and hands its
//! writer to the pipeline as a `RecordSender` (`VideoRoute::set_record`); a
//! failure is named in `STATS` (`tcp_failure`) and goes to the viewer as
//! `VIDEO_TCP_RESULT`. The link's reader watches for its end, which hands
//! the pictures back to the QUIC streams.
//!
//! Viewer: gathers at the session's start (`ViewerTcp::gather`), sends the
//! offer, and on the host's answer establishes the link in a task of its
//! own that passes every record (one picture message, as long as the frame
//! contract lets a picture be: a lossless tile picture too) to the picture
//! task and tells the host `VIDEO_TCP_RESULT`. Its writer stays open while
//! the link carries pictures: its end would end the host's carrier.
//!
//! The Iroh endpoint's own addresses (`GatherPlan::endpoint_addrs`, as the
//! Share worker last published them, QAD included) give the predicted
//! candidate for port-preserving NATs beside the reflected address.
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::screen::core::video_tcp_wire::{
    TcpPathKind, VideoTcpAnswer, VideoTcpOffer, VideoTcpResult, VIDEO_TCP_ANSWER, VIDEO_TCP_RESULT,
};
use crate::screen::core::video_transport::video_route::{RecordSender, SendFuture, VideoRoute};
use crate::screen::core::wire_control::ViewerFrame;
use crate::screen::os::video_tcp_link::{
    GatherPlan, HostTcpLocal, ReflectorTarget, TcpLinkFailure, TcpPictureLink, TcpRecordReader,
    TcpRecordWriter, ViewerTcpLocal, ESTABLISH_TIMEOUT, REFLECT_TIMEOUT,
};

/// One line in the screen log of this process.
fn log(line: &str) {
    let line = format!("screen tcp: {line}");
    // The unit tests write no file of the account they run under.
    #[cfg(not(test))]
    crate::screen::os::host_worker_client::host_log::note(None, &line);
    #[cfg(test)]
    let _ = line;
}

/// Where the candidates come from besides the interfaces.
fn gather_plan() -> GatherPlan {
    let reflector =
        crate::share::address_services::server_address_services().and_then(|services| {
            Some(ReflectorTarget {
                host: services.host,
                port: services.tcp_reflect_port?,
            })
        });
    GatherPlan {
        reflector,
        endpoint_addrs: crate::share::address_services::own_endpoint_addrs(),
    }
}

/// The key of `TcpPathKind` in `STATS` (`tcp_link`).
fn link_key(path: TcpPathKind) -> &'static str {
    match path {
        TcpPathKind::Lan => "lan",
        TcpPathKind::Ipv6 => "ipv6",
        TcpPathKind::Public => "public",
        TcpPathKind::Punched => "punched",
        TcpPathKind::Other => "other",
    }
}

/// The host's writer of the link as the pipeline's record carrier.
struct TcpRecordSender {
    writer: Mutex<TcpRecordWriter>,
    link: &'static str,
}

impl RecordSender for TcpRecordSender {
    fn send<'a>(&'a self, record: &'a [u8]) -> SendFuture<'a> {
        Box::pin(async move { self.writer.lock().await.send(&[record]).await })
    }

    fn link(&self) -> &str {
        self.link
    }
}

async fn host_failed(
    route: &VideoRoute,
    out: &mpsc::Sender<(u8, Vec<u8>)>,
    failure: TcpLinkFailure,
) {
    route.note(&failure.status_line());
    route.set_tcp_failure(Some(failure.status_line()));
    if let Ok(result) = failure.result().encode() {
        let _ = out.send((VIDEO_TCP_RESULT, result)).await;
    }
}

/// The host's side of the link for the viewer's `offer`; runs until the
/// link ends or the session aborts it.
pub fn spawn_host_tcp(
    offer: VideoTcpOffer,
    route: Arc<VideoRoute>,
    out: mpsc::Sender<(u8, Vec<u8>)>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let local =
            match HostTcpLocal::gather(&gather_plan(), Instant::now() + REFLECT_TIMEOUT).await {
                Ok(local) => local,
                Err(failure) => return host_failed(&route, &out, failure).await,
            };
        if let Some(error) = local.reflect_error() {
            route.note(&format!("TCP reflector gave no address: {error}"));
        }
        let answer = match local.answer().encode() {
            Ok(answer) => answer,
            Err(error) => {
                route.note(&format!("VIDEO_TCP_ANSWER not written: {error}"));
                return;
            }
        };
        if out.send((VIDEO_TCP_ANSWER, answer)).await.is_err() {
            return;
        }
        let link = match local
            .establish(&offer, Instant::now() + ESTABLISH_TIMEOUT)
            .await
        {
            Ok(link) => link,
            Err(failure) => return host_failed(&route, &out, failure).await,
        };
        let TcpPictureLink {
            info,
            mut reader,
            writer,
        } = link;
        route.note(&format!(
            "direct TCP link to {} ({})",
            info.remote,
            info.path.label()
        ));
        let connected = VideoTcpResult::Connected {
            remote: info.remote,
            path: info.path,
        };
        if let Ok(result) = connected.encode() {
            let _ = out.send((VIDEO_TCP_RESULT, result)).await;
        }
        route.set_record(Some(Arc::new(TcpRecordSender {
            writer: Mutex::new(writer),
            link: link_key(info.path),
        })));
        // The viewer sends nothing on the link; its end ends the carrier.
        let reason = loop {
            match reader.recv().await {
                Ok(Some(_)) => continue,
                Ok(None) => break "die Gegenstelle hat die TCP-Verbindung beendet".to_string(),
                Err(error) => break error.to_string(),
            }
        };
        route.note(&format!("direct TCP link ended: {reason}"));
        route.record_ended(&reason);
    })
}

/// Passes every record of the viewer's `reader` (one picture message each,
/// at most the reader's bound) to the picture task until the link ends;
/// `Some`: why it ended, `None`: the picture task is gone. A place in
/// `records` is taken before the next record is read, so whole pictures
/// wait only in its places (`video_viewer.rs` `RECORDS`), and a slower
/// decoder holds the host back through TCP's flow control.
async fn forward_pictures(
    reader: &mut TcpRecordReader,
    records: &mpsc::Sender<Vec<u8>>,
) -> Option<String> {
    loop {
        let place = records.reserve().await.ok()?;
        match reader.recv().await {
            Ok(Some(record)) => place.send(record),
            Ok(None) => return Some("direct TCP link ended by the host".to_string()),
            Err(error) => return Some(format!("direct TCP link ended: {error}")),
        }
    }
}

/// The viewer's side of the link.
pub struct ViewerTcp {
    local: Option<ViewerTcpLocal>,
    /// The largest record the reader takes: the largest picture message of
    /// the frame contract (`Admission::max_message`); 0 keeps the default.
    max_record: usize,
}

impl ViewerTcp {
    /// Opens the listeners and gathers the candidates.
    pub async fn gather(max_record: usize) -> Result<Self, TcpLinkFailure> {
        let local =
            ViewerTcpLocal::gather(&gather_plan(), Instant::now() + REFLECT_TIMEOUT).await?;
        if let Some(error) = local.reflect_error() {
            log(&format!("viewer: TCP reflector gave no address: {error}"));
        }
        Ok(Self {
            local: Some(local),
            max_record,
        })
    }

    /// `VIDEO_TCP_OFFER`, until the answer came.
    pub fn offer(&self) -> Option<VideoTcpOffer> {
        self.local.as_ref().map(ViewerTcpLocal::offer)
    }

    /// The host's `VIDEO_TCP_ANSWER`: establishes the link in a task that
    /// passes its records to `records` and its result to the host through
    /// `control`. Once per session.
    pub fn answer(
        &mut self,
        payload: &[u8],
        records: mpsc::Sender<Vec<u8>>,
        control: mpsc::Sender<ViewerFrame>,
    ) -> Option<JoinHandle<()>> {
        let answer = match VideoTcpAnswer::decode(payload) {
            Ok(answer) => answer,
            Err(error) => {
                log(&format!("viewer: VIDEO_TCP_ANSWER unreadable: {error}"));
                return None;
            }
        };
        let local = self.local.take()?;
        let max_record = self.max_record;
        Some(tokio::spawn(async move {
            let deadline = Instant::now() + ESTABLISH_TIMEOUT;
            let link = match local.establish(&answer, deadline).await {
                Ok(link) => link,
                Err(failure) => {
                    log(&format!("viewer: {}", failure.status_line()));
                    let _ = control
                        .send(ViewerFrame::VideoTcpResult(failure.result()))
                        .await;
                    return;
                }
            };
            let TcpPictureLink {
                info,
                mut reader,
                writer,
            } = link;
            log(&format!(
                "viewer: direct TCP link to {} ({})",
                info.remote,
                info.path.label()
            ));
            let connected = VideoTcpResult::Connected {
                remote: info.remote,
                path: info.path,
            };
            let _ = control.send(ViewerFrame::VideoTcpResult(connected)).await;
            if max_record > 0 {
                reader.set_max_record(max_record);
            }
            // Open while the pictures come: its end would end the carrier.
            let _writer = writer;
            if let Some(end) = forward_pictures(&mut reader, &records).await {
                log(&format!("viewer: {end}"));
            }
        }))
    }
}

#[cfg(test)]
#[path = "video_tcp_carrier_tests.rs"]
mod tests;
