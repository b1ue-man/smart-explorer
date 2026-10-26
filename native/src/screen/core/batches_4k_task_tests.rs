//! Native 4K admission and pixel identity, without a live provider/codec account.
use super::frame_budget::*;
use super::frame_protocol::{self,FrameStamp,Operation};
use super::host_backend::{ScreenDecoder,ScreenSource,SourceFrame,TilesScreenDecoder};
use super::pixel_convert::{BgraImage,FrameSize,Rect};
use super::viewer_reconstruct::HybridDecoder;
use super::wire_binary::{self,FrameRecord,GopHeader,StreamKind,FLAG_KEYFRAME,FLAG_MORE};
use super::negotiate::CodecKind;
const SIZE:FrameSize=FrameSize::new(3840,2160);
fn offer()->FrameCapabilities {FrameCapabilities::admitted(FrameSize::new(16384,16384),768*1024*1024).unwrap()}
struct Capture;
impl ScreenSource for Capture {
    fn size(&self)->FrameSize {SIZE}
    fn capture(&mut self,_id:u64,_key:bool)->std::io::Result<SourceFrame> {Ok(SourceFrame::Unchanged)}
}
fn contract()->FrameContract {FrameContract::negotiate(Some(offer()),Some(offer())).unwrap().unwrap()}
fn header(contract:FrameContract)->GopHeader {
    GopHeader {gop_id:1,frame_contract:Some(contract),family_layout_revision:None,
        kind:StreamKind::Preview,codec:CodecKind::Tiles,profile:None,size:SIZE}
}
#[test]
fn batches_task_b6_native_4k_capture_and_rate_policy_keep_full_pixels() {
    let contract=contract();
    let limits=Capture.frame_limits_for_contract(Some(contract));
    assert_eq!(limits.fit(SIZE),SIZE);assert_eq!(limits.working_bytes,768*1024*1024);
    assert_eq!(VERSIONED_WORKING_IMAGES,24);
    assert!(contract.limits.validate_capture().is_err());
    assert!(FrameLimits::LEGACY.validate_capture().is_ok());
    let packet=super::frame_size_caps::PacketBudget::for_contract(Some(contract)).unwrap();
    let image=contract.limits.max_pixels*4;
    assert!(20*image+32*(packet.record_bytes as u64+super::frame_size_caps::RECORD_BUFFER_OVERHEAD as u64)
        +(packet.encoded_bytes as u64-image)<=contract.limits.working_bytes);
    let full=super::rate_control::quality_for(2_000_000,limits.fit(SIZE),60);
    assert_eq!(full.size,SIZE);
    let preview=super::rate_control::quality_for(2_000_000,FrameSize::new(640,360),3);
    let resumed=super::rate_control::adjust_quality(preview,2_000_000,limits.fit(SIZE),60);
    assert_eq!(resumed.size,SIZE);
    assert_eq!(contract.limits.working_bytes,768*1024*1024);
    let choices=super::negotiate::negotiate_for_contract(&[super::negotiate::CodecOffer::tiles()],
        &[super::negotiate::CodecOffer::tiles()],super::negotiate::CodecPreference::Auto,Some(contract));
    assert!(choices.iter().any(|choice|choice.max_size.width>=SIZE.width && choice.max_size.height>=SIZE.height
        && choice.max_pixels>=SIZE.pixels()));
    let small=FrameCapabilities::admitted(offer().texture_size,32*1024*1024).unwrap();
    let small=FrameContract::negotiate(Some(offer()),Some(small)).unwrap().unwrap();
    assert_ne!(small.limits.fit(SIZE),SIZE);
    assert!(!contract_matches(Some(contract),Some(small)));
    assert!(!contract_matches(None,Some(contract)));
    assert!(!contract_matches(Some(contract),None));
}
#[test]
fn batches_task_b6_4k_lossless_correction_retains_every_source_pixel() {
    let contract=contract();
    let quality=super::rate_control::quality_for(2_000_000,Capture.frame_limits_for_contract(Some(contract)).fit(SIZE),60);
    let source=quality.size;assert_eq!(source,SIZE);
    let mut expected=BgraImage::new_with_limits(source,contract.limits).unwrap();
    for (i,pixel) in expected.data_mut().chunks_exact_mut(4).enumerate() {
        let (x,y)=((i%3840) as u32,(i/3840) as u32);
        // Fine glyph-like strokes and coloured single-pixel boundaries expose scaling.
        let stroke=(x%11==0 || y%17==0 || (x+y)%23==0) as u8;
        pixel.copy_from_slice(&[stroke*255,(x%251) as u8,(y%253) as u8,255]);
    }
    let video=Box::new(TilesScreenDecoder::new_with_limits(source,contract.limits).unwrap());
    let mut decoder=HybridDecoder::new(video,source,contract).unwrap();
    let stamp=FrameStamp {generation:1,frame_id:1,base_frame_id:0};
    let record=|op|FrameRecord {frame_id:1,capture_ts_us:100,
        flags:FLAG_KEYFRAME|if matches!(&op,Operation::Commit) {0} else {FLAG_MORE},
        payload:frame_protocol::encode(stamp,op).unwrap()};
    assert!(!decoder.decode(&record(Operation::Begin)).unwrap());
    let mut compressed=0u64;
    for y in (0..2160).step_by(64) {for x in (0..3840).step_by(64) {
        let rect=Rect {x,y,width:64.min(3840-x),height:64.min(2160-y)};
        let correction=record(frame_protocol::lossless(&expected,rect).unwrap());
        compressed+=correction.payload.len() as u64;
        assert!(!decoder.decode(&correction).unwrap());
    }}
    assert!(decoder.decode(&record(Operation::Commit)).unwrap());
    assert_eq!(decoder.picture().size(),source);
    assert_eq!(decoder.picture().data(),expected.data());
    // 2 Mbit/s changes delivery duration, never the negotiated source geometry.
    let minimum_delivery_us=compressed*8*1_000_000/2_000_000;
    assert!(minimum_delivery_us>0);
}
#[test]
fn batches_task_b6_extended_gop_waits_for_real_contract() {
    let header=header(contract());let prefix=header.encode().unwrap();
    assert!(GopHeader::decode(&prefix).is_err());
    let (partial,extended)=GopHeader::decode_prefix(&prefix).unwrap();
    assert!(extended);assert!(partial.frame_contract.is_none());
    let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let mut bytes=Vec::new();wire_binary::write_gop_header(&mut bytes,&header).await.unwrap();
        assert_eq!(wire_binary::read_gop_header(&mut bytes.as_slice()).await.unwrap(),header);
    });
}
