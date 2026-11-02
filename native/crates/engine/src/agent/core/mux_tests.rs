use super::super::lanes::{make_out_channel, run_writer, OUT_BACKLOG};
use super::super::route::{close_transport, route_frame};
use super::Mux;
use crate::agent_proto::{read_frame, Frame, CREDIT_INITIAL, TRANSFER_FRAME_BACKLOG};
use crossbeam_channel::TrySendError;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn mux_with_stall(stall: Duration) -> (Mux, super::super::lanes::OutReceivers) {
    let (out, receivers) = make_out_channel();
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let closed = Arc::new(AtomicBool::new(false));
    (
        Mux::new_with_stall_timeout(out, pending, closed, stall),
        receivers,
    )
}

#[test]
fn remote_drive_task_closing_transport_disconnects_existing_and_new_requests() {
    let (out, _out_rx) = make_out_channel();
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let mux = Mux::new_with_stall_timeout(
        out,
        pending.clone(),
        closed.clone(),
        Duration::from_secs(30),
    );

    let (_, existing) = mux.register();
    close_transport(&closed, &pending);
    assert!(existing.recv().is_err());

    let (id, after_close) = mux.register();
    assert!(after_close.recv().is_err());
    assert!(mux.send(id, Frame::Ok).is_err());
}

#[test]
fn remote_drive_task_registered_stream_is_bounded_and_receiver_drop_unblocks_sender() {
    let (mux, _out_rx) = mux_with_stall(Duration::from_secs(30));
    let (id, receiver) = mux.register();
    let sender = mux.pending.lock().unwrap().get(&id).unwrap().tx.clone();
    for _ in 0..TRANSFER_FRAME_BACKLOG {
        sender.try_send(Frame::Ok).unwrap();
    }
    assert!(matches!(
        sender.try_send(Frame::Ok),
        Err(TrySendError::Full(Frame::Ok))
    ));

    drop(receiver);
    assert!(matches!(
        sender.try_send(Frame::Ok),
        Err(TrySendError::Disconnected(Frame::Ok))
    ));
}

#[test]
fn remote_drive_task_retired_mux_drains_existing_and_rejects_new_requests() {
    let (mux, out_rx) = mux_with_stall(Duration::from_secs(30));
    let (existing_id, existing_rx) = mux.register();

    mux.retire();
    assert!(mux.is_retired());
    assert!(!mux.is_closed());
    mux.send(existing_id, Frame::Ok).unwrap();
    assert_eq!(out_rx.control.recv().unwrap(), (existing_id, Frame::Ok));

    let (new_id, new_rx) = mux.register();
    assert!(new_rx.recv().is_err());
    assert!(mux.send(new_id, Frame::Ok).is_err());
    assert!(!mux.is_closed());

    mux.unregister(existing_id);
    assert!(mux.is_closed());
    assert!(existing_rx.recv().is_err());
}

#[test]
fn remote_drive_task_stalled_writer_queue_times_out_and_disconnects_pending_operations() {
    let (mux, _undrained) = mux_with_stall(Duration::from_millis(25));
    let (id, waiting) = mux.register();
    for _ in 0..OUT_BACKLOG {
        mux.send(id, Frame::Data(vec![1])).unwrap();
    }

    let error = mux.send(id, Frame::Data(vec![1])).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(mux.is_closed());
    assert!(waiting.recv().is_err());
}

#[test]
fn remote_drive_task_unregister_releases_a_router_waiting_on_backpressure() {
    let (mux, _out_rx) = mux_with_stall(Duration::from_secs(30));
    let (id, receiver) = mux.register();
    let sender = mux.pending.lock().unwrap().get(&id).unwrap().tx.clone();
    for _ in 0..TRANSFER_FRAME_BACKLOG {
        sender.try_send(Frame::Ok).unwrap();
    }
    drop(sender);

    let routed = mux.pending.clone();
    let activity = mux.activity();
    let control = mux.out.control.clone();
    let router = std::thread::spawn(move || {
        route_frame(&routed, &activity, &control, Ok(Some((id, Frame::Ok))))
    });
    mux.unregister(id);
    assert!(router.join().unwrap());

    for _ in 0..TRANSFER_FRAME_BACKLOG {
        assert_eq!(receiver.recv().unwrap(), Frame::Ok);
    }
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn transfer_engine_task_credit_route_never_blocks_the_reader_and_enforces_credit() {
    let (mux, out_rx) = mux_with_stall(Duration::from_secs(30));
    mux.enable_credit().unwrap();
    assert_eq!(
        out_rx.control.recv().unwrap(),
        (0, Frame::Credit { bytes: 0 })
    );
    let (id, receiver) = mux.register();
    let activity = mux.activity();
    let chunk = vec![7u8; 64 * 1024];
    let cost = Frame::Data(chunk.clone()).wire_len().unwrap() as u64;
    let fitting = CREDIT_INITIAL / cost;

    // Nobody consumes: the whole initial credit is queued without blocking.
    let started = Instant::now();
    for _ in 0..fitting {
        assert!(route_frame(
            &mux.pending,
            &activity,
            &mux.out.control,
            Ok(Some((id, Frame::Data(chunk.clone()))))
        ));
    }
    assert!(started.elapsed() < Duration::from_secs(1));

    // One frame beyond the credit ends only this request.
    assert!(route_frame(
        &mux.pending,
        &activity,
        &mux.out.control,
        Ok(Some((id, Frame::Data(chunk.clone()))))
    ));
    assert_eq!(out_rx.control.recv().unwrap(), (id, Frame::Cancel));
    for _ in 0..fitting {
        assert!(matches!(receiver.recv().unwrap(), Frame::Data(_)));
    }
    assert!(matches!(receiver.recv().unwrap(), Frame::Err(_)));
    assert!(receiver.recv().is_err());
    assert!(!mux.is_closed());
}

#[test]
fn transfer_engine_task_consumer_returns_credit_and_unfinished_requests_cancel() {
    let (mux, out_rx) = mux_with_stall(Duration::from_secs(30));
    mux.enable_credit().unwrap();
    let _ = out_rx.control.recv().unwrap();
    let (id, receiver) = mux.register();
    let activity = mux.activity();
    let chunk = vec![1u8; 256 * 1024];
    for _ in 0..3 {
        assert!(route_frame(
            &mux.pending,
            &activity,
            &mux.out.control,
            Ok(Some((id, Frame::Data(chunk.clone()))))
        ));
    }
    assert!(matches!(receiver.recv().unwrap(), Frame::Data(_)));
    match out_rx.control.recv_timeout(Duration::from_secs(1)).unwrap() {
        (credit_id, Frame::Credit { bytes }) => {
            assert_eq!(credit_id, id);
            assert!(bytes > 0);
        }
        other => panic!("expected a credit grant, got {other:?}"),
    }
    mux.unregister(id);
    assert_eq!(out_rx.control.recv().unwrap(), (id, Frame::Cancel));

    // A request that saw its terminal reply is not canceled.
    let (done_id, done) = mux.register();
    assert!(route_frame(
        &mux.pending,
        &activity,
        &mux.out.control,
        Ok(Some((done_id, Frame::End)))
    ));
    assert_eq!(done.recv().unwrap(), Frame::End);
    mux.unregister(done_id);
    assert!(out_rx.control.try_recv().is_err());
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<u8>>>);

impl Write for Recorder {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn transfer_engine_task_control_lane_overtakes_queued_upload_data() {
    let (mux, receivers) = mux_with_stall(Duration::from_secs(30));
    let (upload, _upload_rx) = mux.register();
    for _ in 0..4 {
        mux.send(upload, Frame::Data(vec![3; 1024])).unwrap();
    }
    let (listing, _listing_rx) = mux.register();
    mux.send(listing, Frame::ListDir("/".into())).unwrap();

    let recorder = Recorder::default();
    let closed = mux.closed.clone();
    let pending = mux.pending.clone();
    let activity = mux.activity();
    let writer_recorder = recorder.clone();
    let writer = std::thread::spawn(move || {
        run_writer(
            receivers,
            Box::new(writer_recorder),
            closed,
            pending,
            activity,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let frames = loop {
        let bytes = recorder.0.lock().unwrap().clone();
        let mut cursor = io::Cursor::new(bytes);
        let mut frames = Vec::new();
        while let Ok(Some(frame)) = read_frame(&mut cursor) {
            frames.push(frame);
        }
        if frames.len() == 5 || Instant::now() >= deadline {
            break frames;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    mux.close();
    writer.join().unwrap();
    assert_eq!(frames.len(), 5);
    assert_eq!(frames[0], (listing, Frame::ListDir("/".into())));
    assert!(frames[1..]
        .iter()
        .all(|(id, frame)| *id == upload && matches!(frame, Frame::Data(_))));
}
