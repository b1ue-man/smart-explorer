//! Remote-only regression cases: real async/session paths with isolated platform work.
use super::*;
use crate::screen::core::{
    app_icon::IconResponse,
    gop_stream::{accept_picture_streams, PictureEvent},
    host_metadata::{HostMetadata, Request},
    host_metadata_jobs::{self as jobs, Job},
    host_server::{serve, ProvenViewer},
    icon_host::serve_metadata,
    negotiate::CodecOffer,
    registry::StopSelector,
    screen_metadata_runtime_fixtures::{self as f, bounded, Block, Release, Rig},
    window_list::SourceId,
    wire_control::{recv_host_frame, send_viewer_frame, ControlRequest, Ping},
    wire_hello::{recv_hello_reply, HelloReply, ScreenClientHello},
};
use std::sync::atomic::Ordering;

/// The job whose platform step is `name`: a viewer request or the hello metadata.
enum Query {
    Frame(Job<HostFrame>),
    Hello(Job<HostMetadata>),
}
impl Query {
    fn start(rig: &Rig, name: &str) -> Self {
        let request = match name {
            "windows" => Request::Windows,
            "icon" => Request::Icon(rig.backend.icon_request(true)),
            "families" => Request::Families,
            _ => {
                let query = HostMetadata::query(&rig.context, &rig.principal, SessionMode::Probe);
                return Self::Hello(query);
            }
        };
        Self::Frame(request.start(&rig.context, &rig.principal))
    }
    async fn failure(&mut self) -> Option<io::ErrorKind> {
        let result = match self {
            Self::Frame(job) => bounded(job.result()).await.map(drop),
            Self::Hello(job) => bounded(job.result()).await.map(drop),
        };
        result.err().map(|error| error.kind())
    }
}

async fn reply_to(rig: &Rig, request: Request) -> HostFrame {
    let mut job = request.start(&rig.context, &rig.principal);
    bounded(job.result()).await.unwrap()
}

async fn ask(send: &mut SendStream, recv: &mut RecvStream, frame: ViewerFrame) -> HostFrame {
    send_viewer_frame(send, &frame).await.unwrap();
    bounded(recv_host_frame(recv)).await.unwrap()
}

fn viewer(rig: &Rig, link: &f::Link, streams: Streams, hello: ScreenClientHello) -> ProvenViewer {
    let (send, recv) = streams;
    let (connection, principal) = (link.host.clone(), rig.principal.clone());
    let device_name = "Metadata fixture".to_string();
    ProvenViewer {
        connection,
        send,
        recv,
        principal,
        device_name,
        hello,
    }
}
type Streams = (SendStream, RecvStream);

#[test]
fn screen_metadata_runtime_task_queries_preserve_lists_icons_families_and_both_runtimes() {
    for multi in [false, true] {
        f::run(multi, async {
            let rig = f::rig();
            let (context, principal) = (&rig.context, &rig.principal);
            let windows = HostFrame::Windows(rig.backend.list());
            assert_eq!(reply_to(&rig, Request::Windows).await, windows);
            for window in [true, false] {
                let request = rig.backend.icon_request(window);
                let content = Some(rig.backend.reply.clone());
                let expected = HostFrame::IconReply(IconResponse {
                    request: request.clone(),
                    content,
                });
                assert_eq!(reply_to(&rig, Request::Icon(request)).await, expected);
            }
            // A changed icon and a failing platform icon are omitted, not errors.
            let mut stale = rig.backend.icon_request(true);
            stale.icon.sha256 = "0".repeat(64);
            rig.backend.icon_failure.store(true, Ordering::SeqCst);
            for request in [stale, rig.backend.icon_request(false)] {
                let omitted = HostFrame::IconReply(IconResponse {
                    request: request.clone(),
                    content: None,
                });
                assert_eq!(reply_to(&rig, Request::Icon(request)).await, omitted);
            }
            let HostFrame::Families(list) = reply_to(&rig, Request::Families).await else {
                panic!("family reply")
            };
            list.validate().unwrap();
            assert_eq!(list.limitations, ["fixture-family-limit"]);
            let entry = &list.entries[0];
            let session = "1234567890abcdef1234567890abcdef";
            assert_eq!(entry.identity.session, session);
            assert_eq!(entry.identity.generation, 1);
            assert_eq!(
                entry.source.to_string(),
                format!("window:@family.{session}")
            );
            assert_eq!(entry.window, rig.backend.list().windows[0]);
            let launch = entry.launch_request.as_deref();
            assert_eq!(launch, Some("fixture-launch-request"));
            let mut query = HostMetadata::query(context, principal, SessionMode::Probe);
            let metadata = bounded(query.result()).await.unwrap();
            assert_eq!(metadata.monitors[0].id, "fixture-monitor");
            assert_eq!(metadata.encoders, [CodecOffer::tiles()]);
            assert_eq!(metadata.limits, ["fixture-host-limit"]);
            let mut source =
                host_metadata::source_options(context, principal, SessionMode::View, None);
            let source = bounded(source.result()).await.unwrap();
            let monitor = SourceId::Monitor("fixture-monitor".into());
            assert_eq!(source.source, Some(monitor));
            assert!(source.graphical && source.indicator_available);
            assert!(rig.rights.0.lock().is_ok());
        });
    }
}

#[test]
fn screen_metadata_runtime_task_platform_panics_never_poison_share_authorization() {
    f::run(false, async {
        let names = [
            "windows", "icon", "families", "limits", "monitors", "encoders",
        ];
        for name in names {
            let rig = f::rig();
            *rig.backend.panic_at.lock().unwrap() = Some(name);
            let failure = Query::start(&rig, name).failure().await;
            assert_eq!(failure, Some(io::ErrorKind::Other), "{name}");
            assert!(rig.rights.0.lock().is_ok(), "auth mutex after {name} panic");
            let windows = reply_to(&rig, Request::Windows).await;
            assert!(matches!(windows, HostFrame::Windows(_)));
            let view = SessionMode::View.capabilities();
            jobs::check(&rig.context, &rig.principal, view).unwrap();
        }
    });
}

#[test]
fn screen_metadata_runtime_task_revocation_discards_inflight_and_unauthorized_queries() {
    f::run(false, async {
        for name in ["windows", "icon", "families", "limits"] {
            let rig = f::rig();
            let block = Block::new();
            let _release = Release(block.clone());
            *rig.backend.block.lock().unwrap() = Some((name, block.clone()));
            let mut query = Query::start(&rig, name);
            bounded(block.entered.notified()).await;
            f::set_rights(&rig, (false, false)).await;
            block.release();
            let denied = Some(io::ErrorKind::PermissionDenied);
            assert_eq!(query.failure().await, denied, "{name}");
            let calls = rig.backend.calls.load(Ordering::SeqCst);
            assert_eq!(Query::start(&rig, "windows").failure().await, denied);
            assert_eq!(rig.backend.calls.load(Ordering::SeqCst), calls);
        }
    });
}

#[test]
fn screen_metadata_runtime_task_cancelled_work_retains_its_lane_without_starting_queued_queries() {
    f::run(false, async {
        let rig = f::rig();
        let block = Block::new();
        let _release = Release(block.clone());
        *rig.backend.block.lock().unwrap() = Some(("windows", block.clone()));
        let first = Request::Windows.start(&rig.context, &rig.principal);
        bounded(block.entered.notified()).await;
        drop(first);
        *rig.backend.block.lock().unwrap() = None;
        let queued = Request::Families.start(&rig.context, &rig.principal);
        tokio::task::yield_now().await;
        drop(queued);
        let mut next = Request::Windows.start(&rig.context, &rig.principal);
        let early = tokio::time::timeout(Duration::from_millis(20), next.result()).await;
        assert!(early.is_err());
        assert_eq!(rig.backend.calls.load(Ordering::SeqCst), 1);
        block.release();
        assert!(bounded(next.result()).await.is_ok());
        assert_eq!(rig.backend.calls.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn screen_metadata_runtime_task_real_hello_and_metadata_channel_preserve_protocol_without_capture()
{
    f::run(false, async {
        for mode in [SessionMode::Probe, SessionMode::View] {
            let rig = f::rig();
            let link = bounded(f::link()).await;
            let (send, recv, mut viewer_send, mut viewer_recv) = bounded(link.streams()).await;
            let hello = f::hello(mode, mode == SessionMode::View);
            let proven = viewer(&rig, &link, (send, recv), hello);
            let host = tokio::spawn(serve(rig.context.clone(), proven));
            let reply = bounded(recv_hello_reply(&mut viewer_recv)).await.unwrap();
            let HelloReply::Accepted(ok) = reply else {
                panic!("hello accepted")
            };
            assert_eq!(ok.monitors[0].id, "fixture-monitor");
            assert_eq!(ok.limits, ["fixture-host-limit"]);
            if mode == SessionMode::View {
                let (send, recv) = (&mut viewer_send, &mut viewer_recv);
                let windows = ask(send, recv, ViewerFrame::ListWindows).await;
                assert_eq!(windows, HostFrame::Windows(rig.backend.list()));
                for window in [false, true] {
                    let request = ViewerFrame::IconRequest(rig.backend.icon_request(window));
                    let reply = ask(send, recv, request).await;
                    let content = Some(rig.backend.reply.clone());
                    assert!(matches!(reply, HostFrame::IconReply(icon) if icon.content == content));
                }
                let families = ask(send, recv, ViewerFrame::ListFamilies).await;
                assert!(matches!(families, HostFrame::Families(_)));
            }
            assert_eq!(rig.backend.opened.load(Ordering::SeqCst), 0);
            link.viewer.close(0u32.into(), b"fixture complete");
            let _ = bounded(host).await.unwrap();
            assert!(rig.context.registry.is_empty());
            assert!(rig.rights.0.lock().is_ok());
            link.close().await;
        }
    });
}

#[test]
fn screen_metadata_runtime_task_metadata_stop_and_disconnect_discard_running_results() {
    f::run(false, async {
        for disconnect in [false, true] {
            let rig = f::rig();
            let link = bounded(f::link()).await;
            let (send, recv, mut viewer_send, mut viewer_recv) = bounded(link.streams()).await;
            let handle = f::handle(&rig, SessionMode::View);
            let (context, principal) = (rig.context.clone(), rig.principal.clone());
            let connection = link.host.clone();
            let host = tokio::spawn(async move {
                let (context, principal) = (&context, &principal);
                serve_metadata(context, principal, &connection, send, recv, handle, true).await
            });
            let block = Block::new();
            let _release = Release(block.clone());
            *rig.backend.block.lock().unwrap() = Some(("windows", block.clone()));
            let list = ViewerFrame::ListWindows;
            send_viewer_frame(&mut viewer_send, &list).await.unwrap();
            bounded(block.entered.notified()).await;
            if disconnect {
                link.viewer.close(0u32.into(), b"viewer withdrew");
            } else {
                let host_stopped = EndReason::HostStopped;
                rig.context.registry.stop(&StopSelector::All, host_stopped);
            }
            let ended = bounded(host).await.unwrap();
            // A stop ends the channel as before; a vanished viewer is an error.
            assert_eq!(ended.is_ok(), !disconnect);
            assert!(rig.context.registry.is_empty());
            block.release();
            assert!(bounded(recv_host_frame(&mut viewer_recv)).await.is_err());
            assert!(rig.rights.0.lock().is_ok());
            link.close().await;
        }
    });
}

#[test]
fn screen_metadata_runtime_task_live_viewer_keeps_pictures_ping_and_control_release() {
    f::run(true, async {
        let rig = f::rig();
        let link = bounded(f::link()).await;
        let (send, recv, mut viewer_send, mut viewer_recv) = bounded(link.streams()).await;
        let hello = f::hello(SessionMode::Control, false);
        let proven = viewer(&rig, &link, (send, recv), hello);
        let host = tokio::spawn(serve(rig.context.clone(), proven));
        let reply = bounded(recv_hello_reply(&mut viewer_recv)).await.unwrap();
        assert!(matches!(reply, HelloReply::Accepted(_)));
        assert!(rig.context.registry.controller().is_some());
        let (controls, mut outgoing) = mpsc::channel(64);
        let writer = tokio::spawn(async move {
            while let Some(frame) = outgoing.recv().await {
                if send_viewer_frame(&mut viewer_send, &frame).await.is_err() {
                    break;
                }
            }
        });
        let (events, mut pictures) = mpsc::channel(64);
        let reader = accept_picture_streams(link.viewer.clone(), events, controls.clone());
        let reader = tokio::spawn(reader);
        let block = Block::new();
        let _release = Release(block.clone());
        *rig.backend.block.lock().unwrap() = Some(("windows", block.clone()));
        controls.send(ViewerFrame::ListWindows).await.unwrap();
        bounded(block.entered.notified()).await;
        let ping = Ping {
            sequence: 71,
            sent_us: 73,
        };
        let release = ControlRequest {
            action: ControlAction::Release,
        };
        for frame in [
            ViewerFrame::Ping(ping),
            ViewerFrame::ControlRequest(release),
        ] {
            controls.send(frame).await.unwrap();
        }
        let (mut pong, mut released) = (false, false);
        bounded(async {
            while !pong || !released {
                match recv_host_frame(&mut viewer_recv).await.unwrap() {
                    HostFrame::Pong(reply) => pong = reply.sequence == 71,
                    HostFrame::ControlState(state) => released = state.holder.is_none(),
                    HostFrame::Windows(_) => panic!("blocked metadata completed early"),
                    HostFrame::Error(error) => panic!("viewer failed: {error:?}"),
                    _ => {}
                }
            }
        })
        .await;
        assert!(rig.context.registry.controller().is_none());
        // Pictures keep flowing while the window list is still blocked.
        let mut frames = std::collections::BTreeSet::new();
        bounded(async {
            while frames.len() < 3 {
                if let Some(PictureEvent::Record { record, .. }) = pictures.recv().await {
                    if !record.has_more() {
                        frames.insert(record.frame_id);
                    }
                }
            }
        })
        .await;
        block.release();
        bounded(async {
            loop {
                if let HostFrame::Windows(list) = recv_host_frame(&mut viewer_recv).await.unwrap() {
                    assert_eq!(list, rig.backend.list());
                    break;
                }
            }
        })
        .await;
        // Metadata answers keep their FIFO order.
        let expected: Vec<_> = (0..8)
            .map(|index| rig.backend.icon_request(index % 2 == 0))
            .collect();
        for request in &expected {
            let icon = ViewerFrame::IconRequest(request.clone());
            controls.send(icon).await.unwrap();
        }
        controls.send(ViewerFrame::ListFamilies).await.unwrap();
        let (mut icons, mut family) = (0, false);
        bounded(async {
            while !family {
                match recv_host_frame(&mut viewer_recv).await.unwrap() {
                    HostFrame::IconReply(reply) => {
                        assert_eq!(reply.request, expected[icons]);
                        assert_eq!(reply.content, Some(rig.backend.reply.clone()));
                        icons += 1;
                    }
                    HostFrame::Families(list) => {
                        assert_eq!(icons, expected.len());
                        family = list.validate().is_ok();
                    }
                    HostFrame::Error(error) => panic!("viewer failed: {error:?}"),
                    _ => {}
                }
            }
        })
        .await;
        // A stop with revoked rights discards the running and the queued icon.
        let stopped = Block::new();
        let _release_stopped = Release(stopped.clone());
        *rig.backend.block.lock().unwrap() = Some(("icon", stopped.clone()));
        for window in [true, false] {
            let icon = ViewerFrame::IconRequest(rig.backend.icon_request(window));
            controls.send(icon).await.unwrap();
        }
        bounded(stopped.entered.notified()).await;
        let calls = rig.backend.calls.load(Ordering::SeqCst);
        f::set_rights(&rig, (false, false)).await;
        let narrowed = EndReason::CapabilityNarrowed;
        rig.context.registry.stop(&StopSelector::All, narrowed);
        bounded(host).await.unwrap().unwrap();
        bounded(async {
            while let Ok(frame) = recv_host_frame(&mut viewer_recv).await {
                let published = matches!(
                    frame,
                    HostFrame::Windows(_) | HostFrame::IconReply(_) | HostFrame::Families(_)
                );
                assert!(!published, "metadata published after the stop");
            }
        })
        .await;
        stopped.release();
        *rig.backend.block.lock().unwrap() = None;
        f::set_rights(&rig, (true, true)).await;
        let windows = reply_to(&rig, Request::Windows).await;
        assert!(matches!(windows, HostFrame::Windows(_)));
        let after = rig.backend.calls.load(Ordering::SeqCst);
        assert_eq!(after, calls + 1, "queued icon work ended with its viewer");
        reader.abort();
        writer.abort();
        assert!(rig.context.registry.is_empty());
        assert!(rig.rights.0.lock().is_ok());
        link.close().await;
    });
}
