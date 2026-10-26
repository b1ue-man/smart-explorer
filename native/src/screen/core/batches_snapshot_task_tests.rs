//! Snapshot uses the same capture/encoder/header functions as the live pipeline.
use super::*;
use crate::screen::core::{
    frame_budget::{contract_matches, FrameCapabilities, MAX_WORKING_BYTES},
    frame_memory::MemoryPool,
    host_backend::{ScreenDecoder, TilesScreenDecoder},
    pixel_convert::{downscale_bgra_with_limits, BgraImage},
    wire_binary::{FrameRecord, FLAG_KEYFRAME, FLAG_MORE},
};
use std::sync::{Arc, Mutex};

struct Source {
    size: FrameSize,
    family: Option<u64>,
    grant: crate::screen::core::capture_admission::CaptureGrant,
}
impl ScreenSource for Source {
    fn size(&self) -> FrameSize {
        self.size
    }
    fn capture_grant(&self) -> Option<crate::screen::core::capture_admission::CaptureGrant> {
        Some(self.grant.clone())
    }
    fn family_layout_revision(&self) -> Option<u64> {
        self.family
    }
    fn capture(&mut self, _id: u64, _key: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Frame(
            crate::screen::core::capture_test_support::image(&pixels(self.size), &self.grant),
        ))
    }
}
fn pixels(size: FrameSize) -> BgraImage {
    let mut image = BgraImage::new(size).unwrap();
    for (i, pixel) in image.data_mut().chunks_exact_mut(4).enumerate() {
        pixel.copy_from_slice(&[
            (i % 251) as u8,
            (i / 7 % 253) as u8,
            (i / 19 % 255) as u8,
            255,
        ]);
    }
    image
}
#[test]
fn batches_task_b6_snapshot_preserves_accepted_limits_family_and_legacy_pixels() {
    let source_size = FrameSize::new(1280, 720);
    let caps = FrameCapabilities::admitted(FrameSize::new(1024, 1024), 32 * 1024 * 1024).unwrap();
    let modern = FrameContract::negotiate(Some(caps), Some(caps)).unwrap();
    for accepted in [modern, None] {
        let mut stage = Stage {
            source: Box::new(Source {
                size: source_size,
                family: accepted.map(|_| 7),
                grant: crate::screen::core::capture_test_support::grant(source_size, accepted),
            }),
            encoder: None,
            encoder_book: None,
            memory: None,
        };
        let shot = still_tiles(&mut stage, 4, FrameSize::MAX, accepted).unwrap();
        let header = shot.header(9);
        header.encode().unwrap();
        assert_eq!(header.family_layout_revision, accepted.map(|_| 7));
        assert!(contract_matches(header.frame_contract, accepted));
        assert!(header
            .frame_contract
            .is_none_or(|contract| !contract.hybrid_regions));
        let limits = accepted.map_or(FrameLimits::LEGACY, |contract| contract.limits);
        assert_eq!(shot.size, limits.fit(source_size));
        if accepted.is_some() {
            assert_ne!(shot.size, source_size);
        }
        let budget = PacketBudget::for_contract(header.frame_contract).unwrap();
        assert!(shot
            .records
            .iter()
            .all(|record| record.len() <= budget.record_bytes));
        let expected =
            downscale_bgra_with_limits(&pixels(source_size).view(), shot.size, limits).unwrap();
        let mut decoder = TilesScreenDecoder::new_with_limits(shot.size, limits).unwrap();
        let count = shot.records.len();
        let mut records = shot.records;
        records.reverse();
        for index in 0..count {
            let last = index + 1 == count;
            let payload = records.pop().unwrap().unwrap();
            let (data, lease) = payload.into_parts();
            let record = FrameRecord {
                frame_id: 4,
                capture_ts_us: shot.capture_ts_us,
                flags: FLAG_KEYFRAME | if last { 0 } else { FLAG_MORE },
                payload: data,
            };
            assert_eq!(decoder.decode(&record).unwrap(), last);
            drop(record);
            drop(lease);
        }
        assert_eq!(decoder.picture().data(), expected.data());
    }
}

struct ResultOwner {
    records: Vec<Vec<u8>>,
    pool: Arc<MemoryPool>,
    order: Arc<Mutex<Vec<&'static str>>>,
}
impl Drop for ResultOwner {
    fn drop(&mut self) {
        assert!(!self.records.is_empty());
        assert!(self.pool.reserve(None, 1024 * 1024).is_err());
        self.order.lock().unwrap().push("records");
    }
}
struct SourceOwner {
    pool: Arc<MemoryPool>,
    order: Arc<Mutex<Vec<&'static str>>>,
}
impl ScreenSource for SourceOwner {
    fn size(&self) -> FrameSize {
        FrameSize::new(1, 1)
    }
    fn capture(&mut self, _id: u64, _key: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Unchanged)
    }
}
impl Drop for SourceOwner {
    fn drop(&mut self) {
        assert!(self.pool.reserve(None, 1024 * 1024).is_err());
        self.order.lock().unwrap().push("source");
    }
}
#[test]
fn batches_task_b6_abandoned_capture_output_retires_records_before_lease() {
    let pool = Arc::new(MemoryPool::default());
    let order = Arc::new(Mutex::new(Vec::new()));
    let memory = Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap());
    let stage = Stage {
        source: Box::new(SourceOwner {
            pool: pool.clone(),
            order: order.clone(),
        }),
        encoder: None,
        encoder_book: None,
        memory: Some(memory),
    };
    let result = ResultOwner {
        records: vec![vec![0; 64]],
        pool: pool.clone(),
        order: order.clone(),
    };
    // Exactly the output container dropped when a blocking JoinHandle is abandoned.
    drop(StageOutput { result, stage });
    assert_eq!(*order.lock().unwrap(), vec!["records", "source"]);
    assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
}

async fn endpoints() -> (iroh::Endpoint, iroh::Endpoint, Connection, Connection) {
    async fn endpoint() -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup()
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .alpns(vec![b"b6-snapshot-fixture/1".to_vec()])
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
    let (outgoing, incoming) =
        tokio::join!(left.connect(address, b"b6-snapshot-fixture/1"), async {
            right.accept().await.unwrap().await
        });
    (left, right, outgoing.unwrap(), incoming.unwrap())
}
async fn sent(pipeline: &mut Pipeline, connection: &Connection) {
    while pipeline.tick(connection).await.unwrap() != super::super::Produced::Sent {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
async fn received(
    events: &mut tokio::sync::mpsc::Receiver<crate::screen::core::gop_stream::PictureEvent>,
    kind: StreamKind,
) -> (GopHeader, FrameRecord) {
    loop {
        if let crate::screen::core::gop_stream::PictureEvent::Record { header, record, .. } =
            events.recv().await.expect("picture reader remains open")
        {
            if header.kind == kind && !record.has_more() {
                return (header, record);
            }
        }
    }
}
#[test]
fn batches_task_b6_snapshot_returns_to_new_live_keyframe_without_stale_presentation() {
    use crate::screen::core::{
        gop_stream::accept_picture_streams_with_contract,
        negotiate::{negotiate_for_contract, CodecOffer, CodecPreference},
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            let (left, right, host, viewer) = endpoints().await;
            let size = FrameSize::new(64, 64);
            let mut caps = FrameCapabilities::admitted(size, 32 * 1024 * 1024).unwrap();
            caps.hybrid_regions = false;
            let contract = FrameContract::negotiate(Some(caps), Some(caps)).unwrap();
            let choice = negotiate_for_contract(
                &[CodecOffer::tiles()],
                &[CodecOffer::tiles()],
                CodecPreference::Auto,
                contract,
            )[0];
            let mut pipeline = Pipeline::new(
                Arc::new(crate::screen::os::synthetic_backend::SyntheticBackend),
                Box::new(Source {
                    size,
                    family: Some(7),
                }),
                choice,
                &host,
                30,
                Some(size),
                Arc::new(std::sync::atomic::AtomicU32::new(0)),
            );
            pipeline.set_frame_contract(contract).unwrap();
            let (events, mut incoming) = tokio::sync::mpsc::channel(16);
            let (acks, _ack_rx) = tokio::sync::mpsc::channel(16);
            let reader = tokio::spawn(accept_picture_streams_with_contract(
                viewer.clone(),
                events,
                acks,
                contract,
            ));
            sent(&mut pipeline, &host).await;
            let (old, old_frame) = received(&mut incoming, StreamKind::Video).await;
            assert!(pipeline.on_presented(old.gop_id, old_frame.frame_id));
            assert!(pipeline.input_reference_ready());
            pipeline.snapshot(&host, None).await.unwrap();
            let (shot, shot_frame) = received(&mut incoming, StreamKind::Snapshot).await;
            assert!(shot.gop_id > old.gop_id);
            assert!(contract_matches(shot.frame_contract, contract));
            assert!(!pipeline.input_reference_ready());
            assert!(!pipeline.on_presented(old.gop_id, old_frame.frame_id));
            assert!(!pipeline.on_presented(shot.gop_id, shot_frame.frame_id));
            assert!(pipeline.gop.is_none());
            assert!(pipeline.pacer.keyframe_due());
            sent(&mut pipeline, &host).await;
            let (live, live_frame) = received(&mut incoming, StreamKind::Video).await;
            assert!(live.gop_id > shot.gop_id);
            assert_ne!(live_frame.flags & FLAG_KEYFRAME, 0);
            assert!(pipeline.on_presented(live.gop_id, live_frame.frame_id));
            assert!(pipeline.input_reference_ready());
            drop(pipeline);
            host.close(0u32.into(), b"done");
            viewer.close(0u32.into(), b"done");
            reader.abort();
            let _ = reader.await;
            left.close().await;
            right.close().await;
        })
        .await
        .expect("snapshot-to-live contract deadline");
    });
}
