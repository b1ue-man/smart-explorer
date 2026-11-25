//! „Übertragung“ and „Neu messen“ of the Android viewer (user 2026-11-25):
//! the argument of `screen.viewOpen` and `screen.viewSetDelivery`, and the
//! frames that leave only for a host whose statistics name „Übertragung“.
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::super::run::{open_for_tests, SendFrame};
use super::*;
use crate::screen::core::frame_timing::ChainReport;
use crate::screen::core::wire_hello::SessionMode;

#[test]
fn delivery_task_android_bridge_parses_transport() {
    assert_eq!(delivery_arg(&json!({})).unwrap(), None);
    assert_eq!(delivery_arg(&json!({ "delivery": null })).unwrap(), None);
    for mode in Delivery::ALL {
        let args = json!({ "delivery": mode.key() });
        assert_eq!(delivery_arg(&args).unwrap(), Some(mode));
    }
    for wrong in [json!("udp"), json!(""), json!("Reliable"), json!(1)] {
        let error = delivery_arg(&json!({ "delivery": wrong })).unwrap_err();
        assert_eq!(error.kind, "invalid", "{wrong}");
        assert!(error.message.contains("realtime oder reliable"), "{wrong}");
    }
    // Only statistics that name the mode show a host that takes the frames.
    let mut stats = HostStats::default();
    assert!(!host_runs_delivery(&stats));
    stats.chain = Some(ChainReport::default());
    assert!(!host_runs_delivery(&stats));
    stats.chain = Some(ChainReport {
        delivery: Some(Delivery::Realtime),
        ..ChainReport::default()
    });
    assert!(host_runs_delivery(&stats));
}

#[test]
fn delivery_task_android_bridge_sends_only_to_a_host_that_runs_it() {
    let sent: Arc<Mutex<Vec<ViewerFrame>>> = Arc::default();
    let record_to = sent.clone();
    let send: SendFrame = Arc::new(move |frame: &ViewerFrame| {
        record_to.lock().unwrap().push(frame.clone());
        Ok(())
    });
    let session = open_for_tests(SessionMode::View, Some(send));
    let change = json!({ "session": session.id, "delivery": "reliable" });
    let measure = json!({ "session": session.id });
    // Before statistics that name „Übertragung“: nothing leaves.
    assert_eq!(set_delivery(&change).unwrap(), json!({ "switched": false }));
    assert_eq!(remeasure(&measure).unwrap(), json!({ "sent": false }));
    assert!(sent.lock().unwrap().is_empty());
    // A wrong mode is refused, a missing session is not found.
    let wrong = json!({ "session": session.id, "delivery": "tcp" });
    assert_eq!(set_delivery(&wrong).unwrap_err().kind, "invalid");
    let gone = json!({ "session": 4_000_000, "delivery": "reliable" });
    assert_eq!(set_delivery(&gone).unwrap_err().kind, "not_found");
    session.delivery.store(true, Ordering::SeqCst);
    assert_eq!(set_delivery(&change).unwrap(), json!({ "switched": true }));
    assert_eq!(remeasure(&measure).unwrap(), json!({ "sent": true }));
    assert_eq!(
        *sent.lock().unwrap(),
        [
            ViewerFrame::SetDelivery(Delivery::Reliable),
            ViewerFrame::Remeasure
        ]
    );
    session.close();
}
