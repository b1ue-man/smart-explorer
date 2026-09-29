//! Outgoing frames of one agent connection travel in two lanes. Requests,
//! credit grants, cancellations and heartbeats use the control lane and
//! never queue behind upload data; stream frames use the bounded data lane,
//! which keeps uploads backpressured. Before writing a data frame the writer
//! drains the control lane, so a request frame always precedes its data.
use super::route::{close_transport, PendingMap, RoutedFrame};
use crate::agent_proto::{self, Frame};
use crossbeam_channel::{bounded, unbounded, Receiver, Select, Sender, TryRecvError};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Bound on un-sent outgoing data frames. Provides backpressure for uploads
/// while still pipelining roughly 8 MiB of 256 KiB chunks ahead of the wire.
pub(super) const OUT_BACKLOG: usize = 32;

/// How often an idle writer rechecks whether the transport closed.
const WRITER_POLL: Duration = Duration::from_millis(100);

/// Frames of an upload stream (after its request frame) take the data lane.
pub(super) fn is_data_frame(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::Data(_) | Frame::TreeEntry { .. } | Frame::End | Frame::ItemEnd { .. }
    )
}

#[derive(Clone)]
pub(super) struct OutLanes {
    pub(super) control: Sender<RoutedFrame>,
    pub(super) data: Sender<RoutedFrame>,
}

pub(super) struct OutReceivers {
    pub(super) control: Receiver<RoutedFrame>,
    pub(super) data: Receiver<RoutedFrame>,
}

/// Control frames are few and small (one per request, cancel or grant),
/// so their lane is unbounded; the data lane is bounded by `OUT_BACKLOG`.
pub(super) fn make_out_channel() -> (OutLanes, OutReceivers) {
    let (control, control_rx) = unbounded();
    let (data, data_rx) = bounded(OUT_BACKLOG);
    (
        OutLanes { control, data },
        OutReceivers {
            control: control_rx,
            data: data_rx,
        },
    )
}

enum Next {
    Frame(RoutedFrame),
    Idle,
    Closed,
}

fn next_frame(lanes: &OutReceivers) -> Next {
    match lanes.control.try_recv() {
        Ok(frame) => return Next::Frame(frame),
        Err(TryRecvError::Disconnected) => return Next::Closed,
        Err(TryRecvError::Empty) => {}
    }
    match lanes.data.try_recv() {
        Ok(frame) => Next::Frame(frame),
        Err(TryRecvError::Disconnected) => Next::Closed,
        Err(TryRecvError::Empty) => {
            let mut select = Select::new();
            select.recv(&lanes.control);
            select.recv(&lanes.data);
            let _ = select.ready_timeout(WRITER_POLL);
            Next::Idle
        }
    }
}

fn write_routed(
    mut writer: &mut dyn Write,
    (id, frame): &RoutedFrame,
    activity: &super::mux::Activity,
) -> io::Result<()> {
    agent_proto::write_frame(&mut writer, *id, frame)?;
    activity.touch();
    Ok(())
}

/// Writer thread body: control frames first, data frames in order.
pub(super) fn run_writer(
    lanes: OutReceivers,
    mut writer: Box<dyn Write + Send>,
    closed: Arc<AtomicBool>,
    pending: PendingMap,
    activity: Arc<super::mux::Activity>,
) {
    loop {
        if closed.load(Ordering::Acquire) {
            break;
        }
        let routed = match next_frame(&lanes) {
            Next::Frame(routed) => routed,
            Next::Idle => continue,
            Next::Closed => break,
        };
        if is_data_frame(&routed.1) {
            // A request enqueued before this data frame is visible now:
            // write every pending control frame first.
            let mut failed = false;
            while let Ok(control) = lanes.control.try_recv() {
                if write_routed(&mut *writer, &control, &activity).is_err() {
                    failed = true;
                    break;
                }
            }
            if failed {
                break;
            }
        }
        if write_routed(&mut *writer, &routed, &activity).is_err() {
            break;
        }
    }
    close_transport(&closed, &pending);
}
