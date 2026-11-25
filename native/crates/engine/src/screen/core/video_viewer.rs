//! The viewer's side of the picture carriers (`video_transport.rs`): one
//! task per session reads the QUIC datagrams (sound goes on to the audio
//! relay, `ViewerSession::take_audio`), the sockets of the direct UDP pair
//! (`screen/os/shared/video_udp.rs`, `ViewerLink`) and the records of the
//! direct TCP link (`screen/os/shared/video_tcp_carrier.rs`, `ViewerTcp`),
//! puts each picture's packets together (`video_assembly.rs`) and hands
//! whole pictures to the decode path exactly as the picture streams do
//! (`video_delivery::MessageDelivery`). The host's frames outside
//! `HostFrame` reach it from the control reader: `VIDEO_ANSWER`,
//! `VIDEO_TAIL`, `VIDEO_TCP_ANSWER`, `VIDEO_TCP_RESULT`.
//!
//! A still screen: `VIDEO_TAIL` names the host's last picture; when it is
//! not finished `give_up_us` after the tail came (none of its packets
//! arrived, or too few), it is given up and reported (`FRAME_LOSS`), and
//! the host answers with a continuation or a key frame of the unchanged
//! screen.
use std::time::Duration;

use bytes::Bytes;
use iroh::endpoint::Connection;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::super::audio_wire::AudioPacket;
use super::super::gop_stream::{now_us, PictureEvent};
use super::super::video_tcp_wire::{VideoTcpResult, VIDEO_TCP_ANSWER, VIDEO_TCP_RESULT};
use super::super::wire_control::ViewerFrame;
use super::video_assembly::{Assembled, Assembler};
use super::video_delivery::MessageDelivery;
use super::video_message::Admission;
use super::video_packet::is_video;
use super::{stun_server, Carriers, FrameLoss, VideoOffer, VideoReport, VIDEO_ANSWER, VIDEO_TAIL};
use crate::screen::os::video_tcp_carrier::ViewerTcp;
use crate::screen::os::video_udp::{ensure_firewall_in_background, ViewerLink};

/// Interval of `VIDEO_REPORT` while its counters change.
const REPORT_EVERY: Duration = Duration::from_secs(1);
/// The longest sleep of the task without a deadline.
const IDLE_WAKE_US: u64 = 1_000_000;
/// Records of the TCP link queued ahead of the decode path.
const RECORDS: usize = 4;

/// What the viewer's picture task needs (`viewer_client_tasks.rs`).
pub struct ViewerVideo {
    pub connection: Connection,
    /// The datagram carriers both sides read.
    pub carriers: Carriers,
    /// The host named `video_tcp`: the direct TCP link is set up.
    pub tcp: bool,
    pub events: mpsc::Sender<PictureEvent>,
    pub control: mpsc::Sender<ViewerFrame>,
    pub admission: Admission,
    /// The host's frames outside `HostFrame`, as tag and payload.
    pub host_frames: mpsc::Receiver<(u8, Vec<u8>)>,
    /// Sound datagrams for the audio relay; a full queue drops one.
    pub audio: mpsc::Sender<Bytes>,
}

/// Aborts the task of the TCP link with the picture task.
#[derive(Default)]
struct Owned(Option<JoinHandle<()>>);

impl Owned {
    fn set(&mut self, task: Option<JoinHandle<()>>) {
        if let Some(old) = std::mem::replace(&mut self.0, task) {
            old.abort();
        }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        self.set(None);
    }
}

/// The direct pair and the TCP link as gathered (`None`: not wanted; a
/// TCP failure is the result for the host).
type Gathered = (
    Option<ViewerLink>,
    Option<Result<ViewerTcp, VideoTcpResult>>,
);

/// The host's last picture before a still screen, and when it was named.
#[derive(Clone, Copy)]
struct Tail {
    loss: FrameLoss,
    named_us: u64,
}

fn log(line: &str) {
    let line = format!("screen viewer: {line}");
    // The unit tests write no file of the account they run under.
    #[cfg(not(test))]
    crate::screen::os::host_worker_client::host_log::note(None, &line);
    #[cfg(test)]
    let _ = line;
}

/// The assembler's outputs in order, to the delivery; `false` when the
/// session's channels closed.
async fn hand_on(out: Vec<Assembled>, delivery: &mut MessageDelivery, now: u64) -> bool {
    for assembled in out {
        let open = match assembled {
            Assembled::Complete {
                gop_id,
                frame_id,
                message,
                ..
            } => {
                delivery
                    .deliver(&message, Some((gop_id, frame_id)), now)
                    .await
            }
            Assembled::GaveUp {
                gop_id, frame_id, ..
            } => delivery.gave_up(gop_id, frame_id).await,
        };
        if !open {
            return false;
        }
    }
    true
}

/// Gathers the direct pair's and the TCP link's candidates while the sound
/// goes on (no picture packet comes before the offers); `None`: the
/// connection ended.
async fn gather(
    connection: &Connection,
    audio: &mpsc::Sender<Bytes>,
    (udp, tcp, max_message): (bool, bool, usize),
) -> Option<Gathered> {
    let both = async {
        let udp = async {
            if !udp {
                return None;
            }
            ViewerLink::gather(stun_server()).await.ok()
        };
        let tcp = async {
            if !tcp {
                return None;
            }
            Some(ViewerTcp::gather(max_message).await.map_err(|failure| {
                log(&failure.status_line());
                failure.result()
            }))
        };
        tokio::join!(udp, tcp)
    };
    tokio::pin!(both);
    loop {
        tokio::select! {
            gathered = &mut both => return Some(gathered),
            datagram = connection.read_datagram() => match datagram {
                Ok(bytes) if AudioPacket::is_audio(&bytes) => {
                    let _ = audio.try_send(bytes);
                }
                Ok(_) => {}
                Err(_) => return None,
            },
        }
    }
}

/// Runs until the connection or the session's channels end.
pub async fn run(video: ViewerVideo) {
    let ViewerVideo {
        connection,
        carriers,
        tcp,
        events,
        control,
        admission,
        mut host_frames,
        audio,
    } = video;
    // The firewall admits the session's own sockets; never waited for.
    ensure_firewall_in_background("viewer");
    let max_message = admission.max_message().unwrap_or(0);
    let wanted = (carriers.udp, tcp, max_message);
    let Some((mut link, tcp_side)) = gather(&connection, &audio, wanted).await else {
        return;
    };
    if carriers.any() {
        let offered = Carriers {
            datagrams: carriers.datagrams,
            udp: link.is_some(),
        };
        let offer = VideoOffer {
            carriers: offered.names(),
            candidates: link
                .as_ref()
                .map(|link| link.candidates())
                .unwrap_or_default(),
        };
        if control.send(ViewerFrame::VideoOffer(offer)).await.is_err() {
            return;
        }
    }
    let mut tcp_side = match tcp_side {
        Some(Ok(side)) => {
            if let Some(offer) = side.offer() {
                if control
                    .send(ViewerFrame::VideoTcpOffer(offer))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Some(side)
        }
        Some(Err(failed)) => {
            let _ = control.send(ViewerFrame::VideoTcpResult(failed)).await;
            None
        }
        None => None,
    };
    let mut assembler = Assembler::new(max_message);
    let mut delivery = MessageDelivery::new(events, control.clone(), admission);
    let mut sockets = link.as_ref().map(|link| link.sockets());
    let (records_tx, mut records) = mpsc::channel::<Vec<u8>>(RECORDS);
    let mut tcp_task = Owned::default();
    let mut frames_open = true;
    let mut records_open = true;
    let mut tail: Option<Tail> = None;
    let mut direct_packets = 0u64;
    let mut reported = VideoReport::default();
    let mut report_due = now_us() + REPORT_EVERY.as_micros() as u64;
    loop {
        let now = now_us();
        let tail_due = tail.map(|tail| tail.named_us + assembler.give_up_us());
        let wake = [
            assembler.deadline_us(),
            link.as_ref().and_then(|link| link.due_us()),
            tail_due,
            Some(report_due),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(now + IDLE_WAKE_US);
        let sleep = Duration::from_micros(wake.saturating_sub(now));
        let from_socket = async {
            match &sockets {
                Some(sockets) => sockets.recv().await,
                None => std::future::pending().await,
            }
        };
        let mut packet: Option<(Vec<u8>, bool)> = None;
        let mut record: Option<Vec<u8>> = None;
        tokio::select! {
            datagram = connection.read_datagram() => match datagram {
                Ok(bytes) if AudioPacket::is_audio(&bytes) => {
                    let _ = audio.try_send(bytes);
                }
                Ok(bytes) if is_video(&bytes) => packet = Some((bytes.to_vec(), false)),
                Ok(_) => {}
                Err(_) => return,
            },
            got = from_socket => {
                if let Some(active) = link.as_mut() {
                    let (inner, usable) = active.on_datagram(got, now_us()).await;
                    // Sockets that keep failing: the QUIC datagrams remain.
                    if !usable {
                        link = None;
                        sockets = None;
                    }
                    packet = inner.filter(|inner| is_video(inner)).map(|inner| (inner, true));
                }
            },
            got = records.recv(), if records_open => match got {
                Some(bytes) => record = Some(bytes),
                None => records_open = false,
            },
            frame = host_frames.recv(), if frames_open => match frame {
                Some((VIDEO_ANSWER, payload)) => {
                    if let Some(active) = link.as_mut() {
                        let _ = active.answer(&payload, now_us());
                    }
                }
                Some((VIDEO_TAIL, payload)) => {
                    if let Ok(loss) = FrameLoss::decode(&payload) {
                        tail = Some(Tail { loss, named_us: now_us() });
                    }
                }
                Some((VIDEO_TCP_ANSWER, payload)) => {
                    if let Some(side) = tcp_side.as_mut() {
                        let records = records_tx.clone();
                        tcp_task.set(side.answer(&payload, records, control.clone()));
                    }
                }
                Some((VIDEO_TCP_RESULT, payload)) => match VideoTcpResult::decode(&payload) {
                    Ok(VideoTcpResult::Failed { reason }) => log(&format!(
                        "host: TCP direkt nicht möglich: {}",
                        reason.text()
                    )),
                    Ok(VideoTcpResult::Connected { remote, .. }) => {
                        log(&format!("host reached the direct TCP link from {remote}"));
                    }
                    Err(_) => {}
                },
                Some(_) => {}
                None => frames_open = false,
            },
            () = tokio::time::sleep(sleep) => {}
        }
        let now = now_us();
        if let Some(bytes) = record {
            if !delivery.deliver(&bytes, None, now).await {
                return;
            }
        }
        if let Some((bytes, direct)) = packet {
            direct_packets += u64::from(direct);
            let out = assembler.push(&bytes, now);
            if !hand_on(out, &mut delivery, now).await {
                return;
            }
        }
        // Waiting for the decoder is no silence of the line: the packets of
        // that time are still unread (`Assembler::postpone`).
        let after = now_us();
        let stalled = after.saturating_sub(now);
        assembler.postpone(stalled);
        if let Some(named) = tail.as_mut() {
            named.named_us = named.named_us.saturating_add(stalled);
        }
        let now = after;
        let expired = assembler.expire(now);
        if !hand_on(expired, &mut delivery, now).await {
            return;
        }
        if let Some(named) = tail {
            let loss = named.loss;
            if assembler.finished(loss.frame_id) {
                tail = None;
            } else if now >= named.named_us + assembler.give_up_us() {
                tail = None;
                let (out, present) = assembler.abandon_through(loss.frame_id);
                if !hand_on(out, &mut delivery, now).await {
                    return;
                }
                if !present && !delivery.gave_up(loss.gop_id, loss.frame_id).await {
                    return;
                }
            }
        }
        if let Some(active) = link.as_mut() {
            active.tick(now).await;
        }
        if now >= report_due {
            report_due = now + REPORT_EVERY.as_micros() as u64;
            let counters = assembler.counters;
            let report = VideoReport {
                pictures: delivery.pictures,
                lost_pictures: counters.lost_pictures + delivery.unreadable,
                skipped_pictures: delivery.skipped,
                recovered_pictures: counters.recovered_pictures,
                lost_packets: counters.lost_packets,
                recovered_packets: counters.recovered_packets,
                late_packets: counters.late_packets,
                direct_packets,
            };
            if report != reported {
                reported = report;
                if control
                    .send(ViewerFrame::VideoReport(report))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}
