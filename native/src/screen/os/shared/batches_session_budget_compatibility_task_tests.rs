//! Private transport and legacy interoperability cases for the remote suite.
use crate::screen::core::{
    frame_budget::{self, FrameCapabilities},
    frame_memory::{CapacityCeiling, MemoryPool},
    gop_stream::{GopWriter, PictureEvent},
    negotiate::CodecKind,
    pixel_convert::{BgraImage, FrameSize},
    tiles_codec::{TileEncoder, MAX_TILES_RECORD_BYTES},
    viewer_client,
    wire_binary::{GopHeader, StreamKind},
    wire_hello::{self, HelloReply, ScreenClientHello, ScreenHelloOk, ScreenServerHello},
};
use std::{io, sync::Arc, time::Duration};
const MIB: u64 = 1024 * 1024;
fn client() -> ScreenClientHello {
    serde_json::from_value(serde_json::json!({"protocol_version":1,"relation_kind":"direct","relation_id":"fixture",
        "device_id":"viewer","device_name":"viewer","public_key":"key","fingerprint":"pin","node_id":"node",
        "client_nonce":"nonce","proof":"proof","mode":"preview","source":"synthetic",
        "decoders":[{"codec":"tiles"}],"display_size":{"width":320,"height":320},"max_fps":1})).unwrap()
}
fn legacy_accepted() -> ScreenHelloOk {
    serde_json::from_value(
        serde_json::json!({"authorization":{"view":"allow","control":"allow"},
        "platform":"windows","approval":"not_required"}),
    )
    .unwrap()
}

#[test]
fn batches_task_b6_session_budget_android_policy_shares_previews_desktop_keeps_full_contract() {
    use crate::screen::core::frame_budget::PreviewAdmission;
    let server = ScreenServerHello::new(
        "challenge".into(),
        "host".into(),
        "key".into(),
        "pin".into(),
        "node".into(),
    );
    let mut request = client();
    request.frame_capabilities =
        Some(FrameCapabilities::admitted(FrameSize::new(4096, 4096), 64 * MIB).unwrap());
    let pool = Arc::new(MemoryPool::default());
    let ceiling = CapacityCeiling::bounded(64 * MIB);
    let reserve = |bytes| pool.reserve_with_ceiling(None, ceiling, bytes);
    let (first, first_memory) = super::admit_hello(
        request.clone(),
        &server,
        PreviewAdmission::Geometric,
        reserve,
    )
    .unwrap();
    let (second, second_memory) = super::admit_hello(
        request.clone(),
        &server,
        PreviewAdmission::Geometric,
        reserve,
    )
    .unwrap();
    assert!(first_memory.unwrap().bytes() < 32 * MIB);
    assert!(second_memory.unwrap().bytes() < 32 * MIB);
    assert_eq!(
        first.frame_capabilities.unwrap().frames.max_size,
        FrameSize::new(320, 320)
    );
    assert_eq!(
        second.frame_capabilities.unwrap().frames.max_size,
        FrameSize::new(320, 320)
    );
    let desktop = Arc::new(MemoryPool::default());
    let (hello, memory) =
        super::admit_hello(request, &server, PreviewAdmission::FullContract, |bytes| {
            desktop.reserve_with_ceiling(None, ceiling, bytes)
        })
        .unwrap();
    assert_eq!(memory.unwrap().bytes(), 64 * MIB);
    assert!(
        hello
            .frame_capabilities
            .unwrap()
            .frames
            .check(FrameSize::new(960, 540))
            .is_ok(),
        "desktop promotes the same session to full view"
    );
    let mut probe = client();
    probe.mode = crate::screen::core::wire_hello::SessionMode::Probe;
    let (hello, memory) = super::admit_hello(probe, &server, PreviewAdmission::Geometric, |_| {
        panic!("a capability probe must not reserve image memory")
    })
    .unwrap();
    assert!(memory.is_none());
    assert!(hello.frame_capabilities.is_none());
}
async fn endpoints() -> (
    iroh::Endpoint,
    iroh::Endpoint,
    iroh::endpoint::Connection,
    iroh::endpoint::Connection,
) {
    async fn endpoint() -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup()
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .alpns(vec![b"b6-session-budget/1".to_vec()])
            .bind()
            .await
            .unwrap()
    }
    let left = endpoint().await;
    let right = endpoint().await;
    let address = iroh::EndpointAddr::from_parts(
        right.id(),
        right
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );
    let (outgoing, incoming) = tokio::join!(left.connect(address, b"b6-session-budget/1"), async {
        right.accept().await.unwrap().await
    });
    (left, right, outgoing.unwrap(), incoming.unwrap())
}

#[test]
fn batches_task_b6_session_budget_v184_hello_and_picture_work_below_512_mib() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(20), async {
            let (left, right, viewer, host) = endpoints().await;
            let served = tokio::spawn(async move {
                let (mut send, mut recv) = host.open_bi().await.unwrap();
                let mut server = ScreenServerHello::new(
                    "challenge".into(),
                    "host".into(),
                    "key".into(),
                    "pin".into(),
                    "node".into(),
                );
                server.frame_memory_class = None;
                server.family_version = None;
                server.icon_version = None;
                wire_hello::send_server_hello(&mut send, &server)
                    .await
                    .unwrap();
                // The released host ignores the unknown optional resource offer.
                #[derive(serde::Deserialize)]
                struct LegacyClient {
                    display_size: Option<FrameSize>,
                    source: Option<crate::screen::core::window_list::SourceId>,
                }
                let hello: LegacyClient = crate::screen::core::wire::recv_json(
                    &mut recv,
                    crate::screen::core::wire::CLIENT_HELLO,
                )
                .await
                .unwrap();
                assert_eq!(hello.display_size, Some(FrameSize::new(320, 320)));
                assert_eq!(
                    hello.source,
                    Some(crate::screen::core::window_list::SourceId::Synthetic)
                );
                wire_hello::send_hello_reply(&mut send, &HelloReply::Accepted(legacy_accepted()))
                    .await
                    .unwrap();
                let size = FrameSize::new(320, 320);
                let image = BgraImage::new(size).unwrap();
                let records = TileEncoder::new(size)
                    .unwrap()
                    .encode(&image.view(), true, MAX_TILES_RECORD_BYTES)
                    .unwrap();
                let header = GopHeader {
                    gop_id: 7,
                    frame_contract: None,
                    family_layout_revision: None,
                    kind: StreamKind::Preview,
                    codec: CodecKind::Tiles,
                    profile: None,
                    size,
                };
                let mut gop = GopWriter::open(&host, header).await.unwrap();
                gop.write_frame(1, 123, true, records).await.unwrap();
                gop.finish();
                host.closed().await;
            });
            let pool = Arc::new(MemoryPool::default());
            let request = client();
            let wanted = frame_budget::requested_bytes(&request, 64 * MIB).unwrap();
            let lease = pool
                .reserve_with_ceiling(None, CapacityCeiling::bounded(64 * MIB), wanted)
                .unwrap();
            let offered =
                FrameCapabilities::admitted(FrameSize::new(4096, 4096), lease.bytes()).unwrap();
            let mut request = request;
            request.frame_capabilities =
                Some(frame_budget::preview_capabilities(offered, &request).unwrap());
            let mut session = viewer_client::handshake_admitted(
                viewer,
                |_| Ok((request, Some(Arc::new(lease)))),
                |_| {},
            )
            .await
            .unwrap();
            assert!(session.hello.frame_contract.is_none());
            let local = session.local_limits.unwrap();
            assert!(local.working_bytes < 64 * MIB);
            assert!(local.check(FrameSize::new(320, 320)).is_ok());
            let renderer = session.memory.clone();
            loop {
                if let PictureEvent::Record { header, record, .. } =
                    session.pictures.recv().await.unwrap()
                {
                    assert!(header.frame_contract.is_none());
                    assert_eq!(header.gop_id, 7);
                    assert_eq!(record.frame_id, 1);
                    assert!(!record.payload.is_empty());
                    break;
                }
            }
            session.close();
            drop(session);
            served.await.unwrap();
            left.close().await;
            right.close().await;
            assert!(
                pool.reserve_with_ceiling(None, CapacityCeiling::bounded(64 * MIB), 64 * MIB)
                    .unwrap()
                    .bytes()
                    < 64 * MIB,
                "the retained consumer still owns its actual lease"
            );
            drop(renderer);
        })
        .await
        .unwrap();
    });
}

#[path = "batches_android_capture_restart_task_tests.rs"]
mod android_capture_restart;

#[test]
fn batches_task_b6_session_budget_android_resume_reuses_capacity_after_actual_owner_end() {
    android_capture_restart::batches_task_b6_session_budget_android_resume_reuses_capacity_after_actual_owner_end();
}

#[test]
fn batches_task_b6_session_budget_android_resume_keeps_live_owner_and_retries_same_generation() {
    android_capture_restart::batches_task_b6_session_budget_android_resume_keeps_live_owner_and_retries_same_generation();
}

#[test]
fn batches_task_b6_session_budget_android_snapshot_replaces_only_retired_or_native_authority() {
    android_capture_restart::batches_task_b6_session_budget_android_snapshot_replaces_only_retired_or_native_authority();
}

#[test]
fn batches_task_b6_session_budget_android_epoch_drops_old_queue_but_keeps_external_owner() {
    android_capture_restart::batches_task_b6_session_budget_android_epoch_drops_old_queue_but_keeps_external_owner();
}
