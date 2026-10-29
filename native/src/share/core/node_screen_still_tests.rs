//! The host's answers to `LIST_WINDOWS` and `SNAPSHOT` between two real
//! Iroh nodes on loopback (SCR-M4), with the test picture as the source:
//! what the relay of the background worker forwards to `se screen windows`,
//! `se screen snapshot` and the MCP tool `screen`. The client's own logic
//! (limits, refusals, assembling) is tested in `screen_ipc_still_tests.rs`.
use super::*;
use crate::screen::core::gop_stream::PictureEvent;
use crate::screen::core::snapshot::{decode_snapshot, fit};
use crate::screen::core::wire_binary::{FrameRecord, StreamKind};
use crate::screen::core::wire_control::{SnapshotRequest, ViewerFrame};

const WAIT: Duration = Duration::from_secs(15);

fn viewer_request() -> ViewerRequest {
    // One picture a second: the video stream of the session stays small.
    request(
        SessionMode::View,
        SourceId::Synthetic,
        FrameSize::new(640, 360),
        1,
    )
}

fn send(pair: &ScreenPair, session: &ViewerSession, frame: ViewerFrame) {
    pair.viewer
        .0
        .block_on(session.control.send(frame))
        .expect("the control stream is open");
}

/// The picture of a `SNAPSHOT` the host answered: its size and its tile
/// records (the video stream of the session is passed over). The other host
/// frames are kept: a host that refuses the request names its `ERROR` in the
/// failure message.
fn still(pair: &ScreenPair, session: &mut ViewerSession) -> (FrameSize, Vec<FrameRecord>) {
    pair.viewer.0.block_on(async {
        let mut frames = Vec::new();
        let collect = async {
            let mut records = Vec::new();
            let mut host_open = true;
            loop {
                tokio::select! {
                    event = session.pictures.recv() => match event {
                        Some(PictureEvent::Record { header, record, .. })
                            if header.kind == StreamKind::Snapshot =>
                        {
                            let more = record.has_more();
                            records.push(record);
                            if !more {
                                return Some((header.size, records));
                            }
                        }
                        Some(_) => {}
                        None => return None,
                    },
                    frame = session.host.recv(), if host_open => match frame {
                        Some(Ok(HostFrame::Stats(_) | HostFrame::Pong(_))) => {}
                        Some(Ok(other)) => frames.push(format!("{other:?}")),
                        Some(Err(error)) => frames.push(format!("stream: {error}")),
                        None => host_open = false,
                    },
                }
            }
        };
        let shot = tokio::time::timeout(WAIT, collect).await.ok().flatten();
        shot.unwrap_or_else(|| panic!("the host sends the still picture: {frames:#?}"))
    })
}

#[test]
fn screen_task_ipc_still_windows_roundtrip() {
    let pair = ScreenPair::new(Allow, Off);
    let mut session = pair.open(&viewer_request(), |_| {}).unwrap();
    send(&pair, &session, ViewerFrame::ListWindows);
    let list = pair
        .viewer
        .0
        .block_on(async {
            let answer = async {
                loop {
                    match session.host.recv().await {
                        Some(Ok(HostFrame::Windows(list))) => return Some(list),
                        Some(Ok(_)) => {}
                        _ => return None,
                    }
                }
            };
            tokio::time::timeout(WAIT, answer).await.ok().flatten()
        })
        .expect("the host answers LIST_WINDOWS");
    // A host with the test picture only lists nothing, and says why.
    assert!(list.windows.is_empty());
    assert!(list.apps.is_empty());
    assert_eq!(
        list.list_source,
        crate::screen::core::window_list::ListSource::Unknown
    );
    assert_eq!(list.limits, ["test_picture_only"]);
    session.close();
}

#[test]
fn screen_task_ipc_still_snapshot_roundtrip() {
    let pair = ScreenPair::new(Allow, Off);
    let mut session = pair.open(&viewer_request(), |_| {}).unwrap();
    // The whole picture of the source.
    send(
        &pair,
        &session,
        ViewerFrame::Snapshot(SnapshotRequest {
            source: None,
            max_size: None,
        }),
    );
    let (size, records) = still(&pair, &mut session);
    assert_eq!(size, FrameSize::new(640, 360));
    let image = decode_snapshot(&records, size).unwrap();
    assert!(
        image.data().iter().any(|byte| *byte != 0 && *byte != 255),
        "the test picture is not blank"
    );
    // A smaller one: the host shrinks it to the long edge the viewer names.
    send(
        &pair,
        &session,
        ViewerFrame::Snapshot(SnapshotRequest {
            source: None,
            max_size: Some(FrameSize::new(320, 200)),
        }),
    );
    let (small_size, small_records) = still(&pair, &mut session);
    assert_eq!(small_size, FrameSize::new(320, 180));
    let small = decode_snapshot(&small_records, small_size).unwrap();
    // Both fit a PNG budget the way `se screen snapshot` and the MCP tool
    // save them.
    let png = fit(&small, Some(160), None).unwrap();
    assert_eq!(png.size, FrameSize::new(160, 90));
    assert_eq!(&png.png[..4], &[0x89, b'P', b'N', b'G']);
    session.close();
}
