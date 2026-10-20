//! The bridge contract of this phone as screen host (SCR-M8): the answers
//! of `screen.host*`, the events `screenHostStart` and `screenSession`, the
//! refusal of bad arguments and the two JNI calls.
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};

use super::{handle, screen_control, screen_push};
use crate::screen::core::host_backend::ScreenHostBackend;
use crate::screen::core::window_list::SourceId;
use crate::screen::os::bridge_backend::BridgeBackend;
use crate::screen::os::bridge_channels::{self as channels, FLAG_KEYFRAME};

static EVENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn collect(event: Value) {
    EVENTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(event);
}

fn call(method: &str, args: Value) -> Result<Value, String> {
    handle(method, &args)
        .expect("a screen.host method")
        .map_err(|error| error.kind.to_string())
}

#[test]
fn screen_task_android_bridge_shapes() {
    let _guard = channels::TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    channels::reset_for_tests();
    EVENTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    assert!(handle("screen.viewOpen", &json!({})).is_none());

    let prereqs = call(
        "screen.hostPrereqs",
        json!({
            "accessibility": "restricted",
            "usageAccess": true,
            "appWindowCapture": false,
            "display": {"width": 1080, "height": 2400},
            "encoders": [{"profile": "high", "hardware": true}, {"profile": "vp8"}],
            "apps": [{"package": "org.example.notes", "label": "Notizen"}],
        }),
    )
    .unwrap();
    assert_eq!(
        prereqs,
        json!({"accessibility": "restricted", "usageAccess": true, "appWindowCapture": false})
    );
    assert_eq!(
        call("screen.hostPrereqs", json!({"accessibility": "maybe"})),
        Err("invalid".into())
    );
    assert_eq!(
        channels::prereqs().encoders,
        vec![("high".to_string(), true)]
    );
    assert_eq!(BridgeBackend.windows().unwrap().apps[0].name, "Notizen");

    channels::set_event_sink(collect);
    let _source = BridgeBackend
        .open_source(&SourceId::Monitor("0".into()), None)
        .unwrap();
    let id = channels::channels()[0].id;
    let events = EVENTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        events[0],
        json!({
            "type": "screenHostStart", "session": id, "source": "monitor:0", "still": false,
            "maxWidth": 1080, "maxHeight": 2400, "viewerName": "", "viewerDeviceId": "",
        })
    );
    assert_eq!(
        events[1],
        json!({"type": "screenSession", "session": id, "state": "consent", "reason": null})
    );

    let ready = json!({"session": id, "width": 1080, "height": 2400,
        "encoder": "c2.qti.avc.encoder", "hardware": true, "projection": true});
    assert_eq!(call("screen.hostReady", ready), Ok(json!({})));
    assert_eq!(
        call("screen.hostReady", json!({"session": id})),
        Err("invalid".into())
    );
    assert_eq!(
        call(
            "screen.hostReady",
            json!({"session": id + 7, "width": 1, "height": 1, "encoder": "x"})
        ),
        Err("not_found".into())
    );
    let sessions = call("screen.hostSessions", json!({})).unwrap();
    assert_eq!(
        sessions,
        json!({"sessions": [{
            "session": id, "source": "monitor:0", "still": false, "viewerName": "",
            "viewerDeviceId": "", "state": "streaming", "width": 1080, "height": 2400,
            "encoder": "c2.qti.avc.encoder", "projection": true,
        }]})
    );

    // The JNI calls: a picture in, the next control message out.
    assert_eq!(screen_push(id, b"\0\0\0\x01", 0, FLAG_KEYFRAME), 0);
    assert_eq!(screen_control(id, Duration::from_millis(1)), None);
    assert_eq!(
        call(
            "screen.hostEnded",
            json!({"session": id, "reason": "Consent Denied"})
        ),
        Err("invalid".into())
    );
    assert_eq!(
        call(
            "screen.hostEnded",
            json!({"session": id, "reason": "host_locked"})
        ),
        Ok(json!({}))
    );
    let stop = screen_control(id, Duration::from_millis(1)).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stop).unwrap(),
        json!({"op": "stop", "reason": "host_locked"})
    );
    assert_eq!(
        call("screen.hostStop", json!({"session": "all"})),
        Ok(json!({"stopped": 1}))
    );
    assert_eq!(call("screen.hostStop", json!({})), Err("invalid".into()));
}

/// R15c: the app reports every running app; nothing past a fixed count is
/// cut off (the bridge call is a JNI string, the viewer's list is bounded by
/// its own frame size in `window_list`).
#[test]
fn screen_task_android_app_list_has_no_count_limit() {
    let _guard = channels::TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    channels::reset_for_tests();
    let apps: Vec<Value> = (0..300)
        .map(|index| json!({"package": format!("org.example.app{index}"), "label": format!("App {index}")}))
        .collect();
    call("screen.hostPrereqs", json!({ "apps": apps })).unwrap();
    assert_eq!(channels::prereqs().apps.len(), 300);
}
