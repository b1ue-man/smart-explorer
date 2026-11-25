//! What the host session and its pipeline share about the carriers of the
//! pictures (`video_transport.rs`): the carriers the viewer offered, the
//! valid direct UDP pair (`DirectSender`, set by the link task of
//! `screen/os/shared/video_udp.rs`), the direct TCP link (`RecordSender`,
//! `screen/os/shared/video_tcp_carrier.rs`) or why it failed, the loss the
//! viewer reported last, its counters and the host's own, the carrier of
//! the last picture and the last picture sent as datagrams (`VIDEO_TAIL`).
//! The pipeline picks the carrier per picture (`carrier`,
//! `record_carrier`); the session fills `STATS` from it (`report_into`).
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;

use super::super::frame_timing::ChainReport;
use super::super::gop_stream::path_of;
use super::super::rate_control::PathKind;
use super::video_packet::{shard_len_for, PACKET_HEAD};
use super::video_sink::MessageCarrier;
use super::{Carriers, FrameLoss, VideoPath, VideoReport};

/// A still screen: the last picture sent as datagrams is named this long
/// after it (`VIDEO_TAIL`) when no newer one followed. Its packets left at
/// once (no pace), so they have arrived or are lost by then.
pub const TAIL_AFTER: Duration = Duration::from_millis(100);

/// The future of one sent packet or record.
pub type SendFuture<'a> = Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

/// The host's valid direct pair: seals and sends one picture packet.
pub trait DirectSender: Send + Sync {
    fn send<'a>(&'a self, packet: &'a [u8]) -> SendFuture<'a>;
    /// The largest picture packet it takes.
    fn max_packet(&self) -> usize;
}

/// The host's direct TCP link: sends one picture message as one record.
pub trait RecordSender: Send + Sync {
    fn send<'a>(&'a self, record: &'a [u8]) -> SendFuture<'a>;
    /// How the link runs (`video_tcp_wire::TcpPathKind` key: `lan`,
    /// `ipv6`, `public`, `punched`).
    fn link(&self) -> &str;
}

/// What carried the last picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureCarrier {
    /// Packets with parity: the direct UDP pair or QUIC datagrams.
    Udp(VideoPath),
    /// Records of the direct TCP link.
    Tcp,
    /// A QUIC uni stream per group; `relay`: through the relay.
    QuicStream { relay: bool },
}

/// What the host counted of its picture datagrams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostCounters {
    pub pictures: u64,
    /// Pictures whose packets the carrier did not take (given up).
    pub failed: u64,
    pub packets: u64,
}

#[derive(Default)]
struct RouteState {
    offered: Carriers,
    direct: Option<Arc<dyn DirectSender>>,
    record: Option<Arc<dyn RecordSender>>,
    tcp_failure: Option<String>,
    loss: Option<FrameLoss>,
    report: Option<VideoReport>,
    host: HostCounters,
    carrier: Option<PictureCarrier>,
    /// The last picture went under „TCP“ (`Delivery::Reliable`).
    reliable: bool,
    /// The last picture sent as datagrams, when it left, and whether
    /// `VIDEO_TAIL` named it already.
    last: Option<(u64, u64, Instant)>,
    tailed: bool,
}

#[derive(Default)]
pub struct VideoRoute {
    state: Mutex<RouteState>,
}

impl VideoRoute {
    fn lock(&self) -> MutexGuard<'_, RouteState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One line in the screen host log.
    pub fn note(&self, line: &str) {
        let line = format!("screen carrier: {line}");
        // The unit tests write no file of the account they run under.
        #[cfg(not(test))]
        crate::screen::os::host_worker_client::host_log::note(None, &line);
        #[cfg(test)]
        let _ = line;
    }

    /// The carriers of the viewer's `VIDEO_OFFER` this host reads.
    pub fn offer(&self, carriers: Carriers) {
        self.lock().offered = carriers;
    }

    pub fn offered(&self) -> Carriers {
        self.lock().offered
    }

    /// The valid direct pair, or `None` when there is none (any more).
    pub fn set_direct(&self, direct: Option<Arc<dyn DirectSender>>) {
        self.lock().direct = direct;
    }

    /// The direct TCP link, or `None` when there is none (any more).
    pub fn set_record(&self, record: Option<Arc<dyn RecordSender>>) {
        let mut state = self.lock();
        if record.is_some() {
            state.tcp_failure = None;
        }
        state.record = record;
    }

    /// Why the direct TCP link did not come up (`TcpLinkFailure`).
    pub fn set_tcp_failure(&self, failure: Option<String>) {
        let mut state = self.lock();
        if state.record.is_none() {
            state.tcp_failure = failure;
        }
    }

    /// The direct TCP link broke or ended: the streams carry the pictures.
    pub fn record_ended(&self, reason: &str) {
        let mut state = self.lock();
        if state.record.take().is_some() {
            state.tcp_failure = Some(format!("TCP-Verbindung beendet: {reason}"));
        }
    }

    /// The carrier of the next picture under „UDP“: the direct pair, else
    /// QUIC datagrams the connection can carry; `None`: the streams.
    pub fn carrier(&self, connection: &Connection) -> Option<MessageCarrier> {
        let state = self.lock();
        if let Some(direct) = state.direct.as_ref().filter(|_| state.offered.udp) {
            return Some(MessageCarrier::Direct(direct.clone()));
        }
        let fits = connection
            .max_datagram_size()
            .and_then(shard_len_for)
            .is_some();
        (state.offered.datagrams && fits).then(|| MessageCarrier::Quic(connection.clone()))
    }

    /// The carrier of the next picture under „TCP“: the direct link;
    /// `None`: the streams.
    pub fn record_carrier(&self) -> Option<MessageCarrier> {
        self.lock().record.clone().map(MessageCarrier::Record)
    }

    /// `FRAME_LOSS` of the viewer; the pipeline takes it (`take_loss`).
    pub fn note_loss(&self, loss: FrameLoss) {
        self.lock().loss = Some(loss);
    }

    pub fn take_loss(&self) -> Option<FrameLoss> {
        self.lock().loss.take()
    }

    pub fn note_report(&self, report: VideoReport) {
        self.lock().report = Some(report);
    }

    /// Picture `frame_id` of group `gop_id` went whole on `carrier` in
    /// `packets` packets (or one record).
    pub fn note_sent(&self, carrier: PictureCarrier, packets: usize, gop_id: u64, frame_id: u64) {
        let mut state = self.lock();
        state.host.pictures += 1;
        state.host.packets += packets as u64;
        state.carrier = Some(carrier);
        state.reliable = carrier == PictureCarrier::Tcp;
        state.last =
            matches!(carrier, PictureCarrier::Udp(_)).then(|| (gop_id, frame_id, Instant::now()));
        state.tailed = false;
    }

    /// A picture the carrier did not take.
    pub fn note_failed(&self) {
        self.lock().host.failed += 1;
    }

    /// The pictures go on the picture streams (again); `reliable`: under
    /// „TCP“.
    pub fn note_streams(&self, relay: bool, reliable: bool) {
        let mut state = self.lock();
        state.carrier = Some(PictureCarrier::QuicStream { relay });
        state.reliable = reliable;
        state.last = None;
    }

    /// The last picture sent as datagrams, once `TAIL_AFTER` passed at
    /// `now` without a newer one; each picture once.
    pub fn take_tail(&self, now: Instant) -> Option<FrameLoss> {
        let mut state = self.lock();
        let (gop_id, frame_id, sent) = state.last?;
        if state.tailed || now.saturating_duration_since(sent) < TAIL_AFTER {
            return None;
        }
        state.tailed = true;
        Some(FrameLoss { gop_id, frame_id })
    }

    pub fn counters(&self) -> HostCounters {
        self.lock().host
    }

    /// The carrier fields of `STATS` (`ChainReport::udp_*`, `tcp_*`): the
    /// carrier of the last picture, why the TCP link is missing, and the
    /// datagram counters once the viewer offered a carrier.
    pub fn report_into(&self, chain: &mut ChainReport) {
        let state = self.lock();
        match state.carrier {
            Some(PictureCarrier::Udp(path)) => chain.udp_path = Some(path.key().to_string()),
            Some(PictureCarrier::Tcp) => {
                chain.tcp_path = Some("direct".to_string());
                chain.tcp_link = state
                    .record
                    .as_ref()
                    .map(|record| record.link().to_string());
            }
            Some(PictureCarrier::QuicStream { relay }) if state.reliable => {
                let path = if relay { "relay" } else { "quic_stream" };
                chain.tcp_path = Some(path.to_string());
            }
            _ => {}
        }
        if state.carrier != Some(PictureCarrier::Tcp) {
            chain.tcp_failure = state.tcp_failure.clone();
        }
        if !state.offered.any() {
            return;
        }
        let report = state.report.unwrap_or_default();
        chain.udp_lost_pictures = Some(report.lost_pictures.saturating_add(state.host.failed));
        chain.udp_skipped_pictures = Some(report.skipped_pictures);
        chain.udp_recovered_pictures = Some(report.recovered_pictures);
        chain.udp_lost_packets = Some(report.lost_packets);
        chain.udp_recovered_packets = Some(report.recovered_packets);
    }
}

/// What the host does with `FRAME_LOSS` (`Pipeline::apply_video_events`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossRecovery {
    /// The loss names a group the host no longer writes: answered already.
    Ignore,
    /// The next group continues from this confirmed picture (B-36).
    Continue(u64),
    /// The next group starts with a key frame.
    KeyFrame,
}

/// The answer to `loss` while the host writes the message group
/// `open_group`: the same recovery as after a stale stream (`ResetFacts`).
pub fn loss_recovery(
    open_group: Option<u64>,
    loss: FrameLoss,
    facts: &super::super::gop_continue::ResetFacts,
) -> LossRecovery {
    if open_group != Some(loss.gop_id) {
        return LossRecovery::Ignore;
    }
    match facts.continue_from() {
        Some(last_good) => LossRecovery::Continue(last_good),
        None => LossRecovery::KeyFrame,
    }
}

/// The path name of QUIC datagrams on `connection`.
pub fn quic_path(connection: &Connection) -> VideoPath {
    match path_of(connection).0 {
        PathKind::Direct => VideoPath::QuicDirect,
        PathKind::Relay => VideoPath::Relay,
    }
}

/// The shard length of `carrier`'s packets now; `None` when its packets
/// are too small or it carries whole records.
pub fn shard_len(carrier: &MessageCarrier) -> Option<usize> {
    let max = match carrier {
        MessageCarrier::Direct(direct) => direct.max_packet(),
        MessageCarrier::Quic(connection) => connection.max_datagram_size()?,
        MessageCarrier::Record(_) => return None,
    };
    shard_len_for(max).filter(|len| len + PACKET_HEAD <= max)
}

#[cfg(test)]
#[path = "video_route_tests.rs"]
mod tests;
