//! Android ticket contract fixtures for the one remote batch suite.
//! LoginImageTaskTest supplies HWUI fixtures with simulated bitmap ownership.
//! This module does not exercise JNI, NDK locks, GlobalRefs or native recycle.
use super::*;
use crate::screen::core::{capture_admission::{CaptureGeneration, SourceKey},
    capture_image::CaptureImageBuilder, frame_budget::FrameLimits,
    frame_memory::MemoryPool};

fn fixture() -> Arc<Pane> {
    let metadata = std::mem::size_of::<LoginBitmapTicket>() as u64;
    let limits = ImageOwnerLimits::new(4 * MAX_IMAGE + 2 * MAX_TARGET + 2 * metadata).unwrap()
        .with_role(ImageRole::ViewerInput, MAX_IMAGE, 2).unwrap()
        .with_role(ImageRole::AndroidBitmap, MAX_IMAGE, 2).unwrap()
        .with_role(ImageRole::RenderSurface, MAX_TARGET, 2).unwrap()
        .with_role(ImageRole::ImageMetadata, metadata, 2).unwrap();
    let images = ImageAdmission::in_test_pool(Arc::new(MemoryPool::default()),
        LocalImagePolicy {facts: None, ceiling: CapacityCeiling::bounded(512 * 1024 * 1024)}, limits);
    Arc::new(Pane {images, binding: Mutex::new(Binding {session: 7, generation: 1, closed: false})})
}
fn frame(pane: &Pane, width: u32, height: u32, id: u64) -> ViewerFrame {
    let source = SourceKey::new("android-login-fixture", "authenticated-viewer").unwrap();
    let mut image = CaptureImageBuilder::new_local(crate::screen::core::pixel_convert::FrameSize::new(width,height),
        FrameLimits::LEGACY, &pane.images, ImageRole::ViewerInput, source, CaptureGeneration::new(1)).unwrap();
    for pixel in image.pixels_mut().chunks_exact_mut(4) {pixel.copy_from_slice(&[3,5,7,255]);}
    ViewerFrame {token: FrameToken {frame_id:id,width,height,digest:"a".repeat(64)}, image:image.freeze()}
}
#[test]
fn android_bitmap_ticket_keeps_raw_until_fill_and_observes_real_stride() {
    let pane = fixture();
    let ticket = admit_frame(pane.clone(), 3, 7, 1, frame(&pane,2,2,1)).unwrap().unwrap();
    assert_eq!(pane.images.live_count(ImageRole::ViewerInput).unwrap(),1);
    assert_eq!(pane.images.live_count(ImageRole::AndroidBitmap).unwrap(),1);
    let mut output = vec![0x99;24];
    ticket.fill_rgba(&mut output,12).unwrap();
    assert_eq!(&output[..8], &[7,5,3,255,7,5,3,255]);
    assert_eq!(&output[8..12], &[0x99;4]);
    assert_eq!(&output[12..20], &[7,5,3,255,7,5,3,255]);
    assert!(ticket.fill_rgba(&mut output,7).is_err());
    let lease = ticket.finish();
    assert_eq!(pane.images.live_count(ImageRole::ViewerInput).unwrap(),0);
    assert_eq!(pane.images.live_count(ImageRole::AndroidBitmap).unwrap(),1);
    drop(lease);
    assert_eq!(pane.images.live_bytes().unwrap(),0);
}
#[test]
fn android_reconnect_and_close_cannot_turn_held_pixels_into_a_third_slot() {
    let pane = fixture();
    let first = admit_frame(pane.clone(),3,7,1,frame(&pane,1,1,1)).unwrap().unwrap().finish();
    let second = admit_frame(pane.clone(),3,7,1,frame(&pane,1,1,2)).unwrap().unwrap().finish();
    {let mut state = pane.binding.lock().unwrap(); state.session = 8; state.generation = 2;}
    assert!(!first.is_current());
    assert!(admit_frame(pane.clone(),3,8,2,frame(&pane,1,1,3)).unwrap().is_none());
    assert_eq!(pane.images.live_count(ImageRole::AndroidBitmap).unwrap(),2);
    drop(first);
    let next = admit_frame(pane.clone(),3,8,2,frame(&pane,1,1,4)).unwrap().unwrap().finish();
    assert!(next.is_current());
    pane.binding.lock().unwrap().closed = true;
    assert!(!next.is_current());
    assert_eq!(pane.images.live_count(ImageRole::AndroidBitmap).unwrap(),2);
    drop(second); drop(next);
    assert_eq!(pane.images.live_bytes().unwrap(),0);
}
#[test]
fn android_ticket_rejects_foreign_claim_and_token_without_bitmap_allocation() {
    let pane = fixture(); let foreign = fixture();
    assert!(admit_frame(pane.clone(),3,7,1,frame(&foreign,1,1,1)).is_err());
    let mut mismatched = frame(&pane,1,1,2); mismatched.token.width = 2;
    assert!(admit_frame(pane.clone(),3,7,1,mismatched).is_err());
    let mut stale = frame(&pane,1,1,3); stale.token.digest = "0".into();
    assert!(admit_frame(pane.clone(),3,7,1,stale).is_err());
    assert_eq!(pane.images.live_count(ImageRole::AndroidBitmap).unwrap(),0);
    assert_eq!(pane.images.live_bytes().unwrap(),0);
}
#[test]
fn android_supported_geometry_remains_the_existing_login_limit() {
    for (width,height) in [(1,1),(3840,2160),(4096,2304),(4096,1),(1,4096)] {
        let pane = fixture();
        let ticket = admit_frame(pane.clone(),3,7,1,frame(&pane,width,height,1)).unwrap().unwrap();
        assert_eq!(ticket.width(),width); assert_eq!(ticket.height(),height);
        assert_eq!(ticket.lease.bytes(),u64::from(width)*u64::from(height)*4);
        drop(ticket); assert_eq!(pane.images.live_bytes().unwrap(),0);
    }
}
#[test]
fn android_heap_offer_is_not_expanded_by_a_second_private_pool() {
    let pool = Arc::new(MemoryPool::default());
    let limits = ImageOwnerLimits::new(64).unwrap().with_role(ImageRole::AndroidBitmap,32,2).unwrap();
    let policy = LocalImagePolicy {facts:None,ceiling:CapacityCeiling::bounded(48)};
    let pane = ImageAdmission::in_test_pool(pool.clone(),policy,limits.clone());
    let simultaneous = ImageAdmission::in_test_pool(pool,policy,limits);
    let first = pane.try_claim(ImageRole::AndroidBitmap,32).unwrap();
    assert!(simultaneous.try_claim(ImageRole::AndroidBitmap,32).is_err());
    let small = simultaneous.try_claim(ImageRole::AndroidBitmap,16).unwrap();
    assert!(pane.try_claim(ImageRole::AndroidBitmap,1).is_err());
    drop(small); drop(first);
    assert!(simultaneous.try_claim(ImageRole::AndroidBitmap,32).is_ok());
}
