//! The tasks of a running viewer session (moved out of `viewer_client.rs`,
//! plan 2026-11-14 P4): the writer of the control stream, the reader of the
//! host's frames, the acceptor of the picture streams (which acknowledges
//! every frame) and the `PING` clock. The writer holds the session's
//! interactive presence (E3): it sees every `SET_MODE` and with it whether
//! the session shows the full view (`gop_stream::PicturePresence`).
use std::io;
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::super::gop_stream::{
    accept_picture_streams_admitted, now_us, picture_channel, PictureEvent, PicturePresence,
};
use super::super::wire::MAX_WINDOWS_BYTES;
use super::super::wire_control::{
    decode_host_frame, send_viewer_frame, HostFrame, Ping, StreamMode, ViewerFrame,
};
use super::super::wire_hello::ScreenHelloOk;
use super::{CONTROL_QUEUE, HIDDEN_PING_INTERVAL, PING_INTERVAL};

/// What a session keeps of its tasks.
pub(super) struct Running {
    pub(super) control: mpsc::Sender<ViewerFrame>,
    pub(super) host: mpsc::Receiver<io::Result<HostFrame>>,
    pub(super) pictures: mpsc::Receiver<PictureEvent>,
    pub(super) tasks: Vec<JoinHandle<()>>,
}

/// What the tasks need of the accepted session.
pub(super) struct Start {
    pub(super) connection: Connection,
    pub(super) send: SendStream,
    pub(super) recv: RecvStream,
    pub(super) probe: bool,
    /// Whether the session counts as interactive (E3), kept by the writer.
    pub(super) presence: PicturePresence,
    pub(super) memory: super::super::frame_memory::MemoryHold,
    pub(super) local_limits: Option<super::super::frame_budget::FrameLimits>,
    /// The host announced the pointer overlay (`cursor_overlay::negotiated`):
    /// only then may `CURSOR_REQUEST` leave the viewer.
    pub(super) cursor_overlay: bool,
    /// What the viewer last asked of the overlay (a reconnect asks again).
    pub(super) cursor_wish: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The host announced `rate_ceiling_v1`: only then may `RATE_CEILING`
    /// leave the viewer (an old host ends the session on an unknown frame).
    pub(super) rate_ceiling: bool,
    /// The host announced `set_decoders_v1`: only then may `SET_DECODERS`
    /// leave the viewer (P1).
    pub(super) set_decoders: bool,
}

pub(super) fn spawn(start: Start, accepted: &ScreenHelloOk) -> Running {
    let Start {
        connection,
        mut send,
        mut recv,
        probe,
        mut presence,
        memory,
        local_limits,
        cursor_overlay,
        cursor_wish,
        rate_ceiling,
        set_decoders,
    } = start;
    let (control, mut outgoing) = mpsc::channel::<ViewerFrame>(CONTROL_QUEUE);
    let (host_tx, host) = mpsc::channel(CONTROL_QUEUE);
    let (pictures_tx, pictures) = picture_channel();
    let icons_supported = accepted.icon_version == Some(super::super::app_icon::ICON_VERSION);
    let pending_icons = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        super::super::app_icon::IconRequest,
        std::time::Instant,
    >::new()));
    let sent_icons = pending_icons.clone();
    let hidden = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reported_hidden = hidden.clone();
    let frame_timing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let frame_timing_supported = frame_timing.clone();
    // P9: `SET_DISPLAY` only to a host that answered `live` (`set_display.rs`).
    let set_display = super::super::set_display::negotiated(Some(accepted));
    // Round 12 rest: `INPUT_RECEIPTS` only to a host that announced them.
    let input_receipts =
        accepted.input_receipts == Some(super::super::input_receipt::INPUT_RECEIPTS_V1);
    let writer = tokio::spawn(async move {
        while let Some(frame) = outgoing.recv().await {
            if let ViewerFrame::Visibility(report) = &frame {
                reported_hidden.store(report.hidden(), std::sync::atomic::Ordering::Release);
            }
            // E3: a preview raised to the full view keeps bulk transfers of
            // this process yielding, a full view lowered to a preview not.
            if let ViewerFrame::SetMode(set) = &frame {
                presence.set_full(set.mode == StreamMode::Full);
            }
            if let ViewerFrame::CursorOverlay(on) = &frame {
                if !cursor_overlay {
                    continue;
                }
                cursor_wish.store(*on, std::sync::atomic::Ordering::Release);
            }
            if matches!(frame, ViewerFrame::RateCeiling(_)) && !rate_ceiling {
                continue;
            }
            if matches!(frame, ViewerFrame::SetDecoders(_)) && !set_decoders {
                continue;
            }
            if matches!(frame, ViewerFrame::SetDisplay(_)) && !set_display {
                continue;
            }
            if matches!(frame, ViewerFrame::InputReceipts(_)) && !input_receipts {
                continue;
            }
            if matches!(frame, ViewerFrame::FrameTiming(_))
                && !frame_timing_supported.load(std::sync::atomic::Ordering::Acquire)
            {
                continue;
            }
            if let ViewerFrame::IconRequest(request) = &frame {
                if !icons_supported {
                    continue;
                }
                let admitted = {
                    let mut pending = sent_icons
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    pending.retain(|_, at| at.elapsed() < Duration::from_secs(30));
                    if pending.len() >= 8 || pending.contains_key(request) {
                        false
                    } else {
                        pending.insert(request.clone(), std::time::Instant::now());
                        true
                    }
                };
                if !admitted {
                    continue;
                }
            }
            if send_viewer_frame(&mut send, &frame).await.is_err() {
                return;
            }
        }
        let _ = send.finish();
    });
    let reader = tokio::spawn(async move {
        // Large frames in parts (round 10, `control_parts.rs`).
        let mut parts = super::super::control_parts::PartAssembler::default();
        loop {
            let frame = match super::super::wire::read_frame(&mut recv, MAX_WINDOWS_BYTES).await {
                Ok((super::super::control_parts::CONTROL_PART, part)) => match parts.push(&part) {
                    Ok(None) => continue,
                    Ok(Some((tag, payload))) => decode_host_frame(tag, &payload),
                    Err(error) => Err(error),
                },
                Ok((tag, payload)) => decode_host_frame(tag, &payload),
                Err(error) => Err(error),
            };
            if let Ok(HostFrame::Stats(stats)) = &frame {
                if stats.frame_timing == Some(super::super::frame_timing::FRAME_TIMING_VERSION) {
                    frame_timing.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            if let Ok(HostFrame::IconReply(response)) = &frame {
                // At most eight correlated icon replies may enter the host-frame queue.
                if pending_icons
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&response.request)
                    .is_none()
                {
                    continue;
                }
            }
            let failed = frame.is_err();
            if host_tx.send(frame).await.is_err() || failed {
                return;
            }
        }
    });
    let streams = if probe {
        drop(pictures_tx);
        tokio::spawn(async {})
    } else {
        tokio::spawn(accept_picture_streams_admitted(
            connection,
            pictures_tx,
            control.clone(),
            accepted.frame_contract,
            memory,
            local_limits,
        ))
    };
    let pings = control.clone();
    let clock = tokio::spawn(async move {
        let mut sequence = 0u64;
        loop {
            sequence += 1;
            let ping = Ping {
                sequence,
                sent_us: now_us(),
            };
            if pings.send(ViewerFrame::Ping(ping)).await.is_err() {
                return;
            }
            let interval = if hidden.load(std::sync::atomic::Ordering::Acquire) {
                HIDDEN_PING_INTERVAL
            } else {
                PING_INTERVAL
            };
            tokio::time::sleep(interval).await;
        }
    });
    Running {
        control,
        host,
        pictures,
        tasks: vec![writer, reader, streams, clock],
    }
}
