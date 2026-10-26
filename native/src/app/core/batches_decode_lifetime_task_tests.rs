//! Production DecodeState must retire decoder ownership before Shared's lease.
use super::super::screen_session::StreamLink;
use super::*;
use crate::screen::core::{
    frame_budget::MAX_WORKING_BYTES, frame_memory::MemoryPool, pixel_convert::BgraImage,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
struct Owner {
    image: BgraImage,
    pool: Arc<MemoryPool>,
    dropped: Arc<AtomicBool>,
}
impl ScreenDecoder for Owner {
    fn name(&self) -> String {
        "lease-probe".into()
    }
    fn decode(&mut self, _record: &FrameRecord) -> io::Result<bool> {
        Ok(false)
    }
    fn picture(&self) -> &BgraImage {
        &self.image
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        assert!(
            self.pool.reserve(None, 1024 * 1024).is_err(),
            "decoder pixels outlived their lease"
        );
        self.dropped.store(true, Ordering::SeqCst);
    }
}
fn state() -> (DecodeState, Arc<MemoryPool>, Arc<AtomicBool>) {
    let pool = Arc::new(MemoryPool::default());
    let dropped = Arc::new(AtomicBool::new(false));
    let shared = Shared::new(false, 0);
    *shared.memory.lock().unwrap() = Some(Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap()));
    let mut state = DecodeState::new(
        shared,
        Arc::new(|_| Err(io::Error::other("unused decoder factory"))),
        Arc::new(|_| Ok(())),
        false,
    );
    state.gop = Some(Gop {
        id: 1,
        yuv: false,
        pending: BTreeMap::new(),
        decoder: Box::new(Owner {
            image: BgraImage::new(FrameSize::new(2, 2)).unwrap(),
            pool: pool.clone(),
            dropped: dropped.clone(),
        }),
    });
    (state, pool, dropped)
}
#[test]
fn batches_task_b6_decoder_end_loss_and_drop_keep_last_image_under_lease() {
    for end in [0, 1, 2] {
        let (mut state, pool, dropped) = state();
        match end {
            0 => assert!(state.on_event(StreamEvent::End(
                serde_json::from_value(serde_json::json!({"code":"viewer_closed","detail":null}))
                    .unwrap()
            ))),
            1 => state.connection_lost(),
            _ => {}
        }
        if end < 2 {
            assert!(state.gop.is_none());
            assert!(dropped.load(Ordering::SeqCst));
        }
        drop(state);
        assert!(dropped.load(Ordering::SeqCst));
        assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
    }
}
#[test]
fn batches_task_b6_blocked_next_owns_lease_after_shared_status_anchor_is_removed() {
    let pool = Arc::new(MemoryPool::default());
    let shared = Shared::new(false, 0);
    let lease = Arc::new(Mutex::new(Some(Arc::new(
        pool.reserve(None, MAX_WORKING_BYTES).unwrap(),
    ))));
    let (entered, observed) = mpsc::channel();
    let (finish, events) = mpsc::channel();
    let events = Arc::new(Mutex::new(Some(events)));
    let mut backend = super::super::screen_viewer_fake::Fake::new(
        crate::share::access_grant::CapabilityDecision::Allow,
    )
    .backend();
    backend.open = Arc::new(move |_, _, _| {
        let memory = lease.lock().unwrap().take();
        let events = events.lock().unwrap().take().unwrap();
        let entered = entered.clone();
        Ok(StreamLink {
            memory,
            next: Box::new(move || {
                let _ = entered.send(());
                events
                    .recv()
                    .map_err(|_| io::Error::other("fixture channel closed"))
            }),
            send: Arc::new(|_| Ok(())),
            close: Arc::new(|| Ok(())),
        })
    });
    let (outbox, inbox) = mpsc::channel();
    let run_shared = shared.clone();
    let target = super::super::screen_viewer_fake::target()
        .to_peer()
        .unwrap();
    let request = super::super::screen_pool::preview_request(&backend.offers, 2, 1000);
    let thread = std::thread::spawn(move || {
        super::super::screen_decode_run::run(
            backend,
            target,
            request,
            run_shared,
            (outbox, inbox),
            false,
        )
    });
    observed.recv().unwrap();
    shared.stop_presentation();
    assert!(shared.memory.lock().unwrap().is_none());
    assert!(pool.reserve(None, 1024 * 1024).is_err());
    finish
        .send(StreamEvent::End(
            serde_json::from_value(serde_json::json!({"code":"viewer_closed","detail":null}))
                .unwrap(),
        ))
        .unwrap();
    thread.join().unwrap();
    assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
}
