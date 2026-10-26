//! Adapter fakes exercise the productive receive barrier and mapping ledger.
//! They are source fixtures for the shared remote suite, not live compositor evidence.
use super::super::super::pipewire_abi::memory::rounded_mapping;
use super::super::super::pipewire_abi::{PwBuffer, SpaBuffer, SpaChunk, SpaData};
use super::super::buffers::fixture;
use super::super::imports::fixture_support::Fixture;
use super::super::{buffers, frame_copy, imports};
use super::*;
use crate::screen::core::host_codec_adapters::{HostCodecPlan, HostContract};
use std::os::fd::AsRawFd;
#[test]
fn batches_task_capture_linux_pipewire_unconfigured_producer_negotiates_before_image_grant() {
    let _serial = fixture::SERIAL.lock().unwrap();
    let f = Fixture::new("pw-initial");
    let flags = imports::control::connection_flags(true);
    assert_eq!(flags & ((1 << 1) | (1 << 2) | (1 << 4) | (1 << 8)), 0); // INACTIVE/MAP/RT/ALLOC.
    let size = FrameSize::new(4, 3);
    let output = HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).unwrap();
    assert!(f.memory.capture_plan(&output).unwrap().is_none());
    let fd = fixture::fd(4096);
    unsafe {
        fixture::add(1, &fd, 3);
    }
    assert_eq!(fixture::observed(), (true, 0));
    assert!(!f.latest.is_capture_active());
    assert!(fixture::library_view(1, 0, 48).is_null());
    f.admit();
    assert_eq!(fixture::observed(), (false, 1));
    f.memory
        .format(Some((super::super::VideoFormat::Bgra, size)))
        .unwrap();
    let plan = f.memory.capture_plan(&output).unwrap().unwrap();
    assert_eq!(plan.owner_limits()[CaptureOwner::NativeSurface as usize], 0);
    assert_eq!(
        plan.owner_limits()[CaptureOwner::SourcePixels as usize],
        144
    );
    assert!(!f.latest.is_capture_active());
    f.memory
        .format(Some((
            super::super::VideoFormat::Bgra,
            FrameSize::new(3840, 2160),
        )))
        .unwrap();
    let native = f.memory.capture_plan(&output).unwrap().unwrap();
    assert_eq!(
        native.owner_limits()[CaptureOwner::SourcePixels as usize],
        3 * 3840 * 2160 * 4
    );
}
#[test]
fn batches_task_capture_linux_pipewire_resume_reenters_a_second_receive_barrier() {
    let _serial = fixture::SERIAL.lock().unwrap();
    let f = Fixture::new("pw-reentry");
    let first = fixture::fd(4096);
    let second = fixture::fd(8192);
    unsafe {
        fixture::add(1, &first, 3);
    }
    fixture::resume_add(2, second.as_raw_fd(), 3);
    f.admit();
    assert_eq!(fixture::observed(), (true, 1));
    assert!(f.latest.import_blocked());
    assert_eq!(f.imports.pending().unwrap().mem_id, 2);
    f.admit();
    assert_eq!(fixture::observed(), (false, 2));
    assert!(!f.latest.import_blocked());
}
#[test]
fn batches_task_capture_linux_pipewire_memptr_chunks_and_metadata_use_prior_whole_map() {
    let _serial = fixture::SERIAL.lock().unwrap();
    let f = Fixture::new("pw-memptr");
    let fd = fixture::fd(4096);
    unsafe {
        fixture::add(1, &fd, 3);
    }
    f.admit();
    let base = f.base();
    assert_eq!(fixture::library_view(1, 128, 48) as usize, base + 128);
    let mut block = SpaData {
        type_: 1,
        flags: 0,
        fd: -1,
        mapoffset: 0,
        maxsize: 48,
        data: (base + 128) as *mut _,
        chunk: base as *mut SpaChunk,
    };
    let mut meta = super::super::super::pipewire_abi::SpaMeta {
        type_: 1,
        size: 32,
        data: (base + 256) as *mut _,
    };
    let mut spa = SpaBuffer {
        n_metas: 1,
        n_datas: 1,
        metas: &mut meta,
        datas: &mut block,
    };
    let mut buffer = PwBuffer {
        buffer: &mut spa,
        user_data: std::ptr::null_mut(),
        size: 0,
        requested: 0,
    };
    unsafe {
        f.imports.validate_buffer(&mut buffer).unwrap();
    }
    let before = f.native_live();
    let mut owners = buffers::BufferOwners::default();
    unsafe {
        owners
            .add(&mut buffer, f.memory.generation(), &f.imports)
            .unwrap();
    }
    owners.remove(&mut buffer);
    assert_eq!(f.native_live(), before);
    assert_eq!(fixture::observed().1, 1);
    block.data = (base + 4090) as *mut _;
    assert!(unsafe { f.imports.validate_buffer(&mut buffer) }.is_err());
}
#[test]
fn batches_task_capture_linux_pipewire_exact_fd_objects_aliases_padding_and_pages() {
    let _serial = fixture::SERIAL.lock().unwrap();
    let f = Fixture::new("pw-alias");
    assert_eq!(rounded_mapping(16385, 16384).unwrap(), 32768);
    assert!(rounded_mapping(u64::MAX, 16384).is_err());
    let fd = fixture::fd(6000);
    unsafe {
        fixture::add(1, &fd, 3);
    }
    f.admit();
    let first = f.native_live();
    let block = SpaData {
        type_: 2,
        flags: 0,
        fd: fd.as_raw_fd() as i64,
        mapoffset: 128,
        maxsize: 256,
        data: std::ptr::null_mut(),
        chunk: std::ptr::null_mut(),
    };
    unsafe {
        f.imports.data_range(&block).unwrap();
        f.imports.data_range(&block).unwrap();
    }
    assert_eq!(f.native_live(), first);
    assert_eq!(fixture::observed().1, 1);
    assert_eq!(
        frame_copy::chunk_span(8, 64, 256, 24, FrameSize::new(4, 3)).unwrap(),
        (8, 64)
    );
    let separate = fixture::fd(4096);
    unsafe {
        fixture::add(2, &separate, 3);
    }
    f.admit();
    assert!(f.native_live() > first);
    assert_eq!(fixture::observed().1, 2);
}
#[test]
fn batches_task_capture_linux_pipewire_consumer_minimum_omits_guessed_count_and_stride() {
    let bytes =
        super::super::super::spa_pod::consumer_buffers_pod(FrameSize::new(3840, 2160)).unwrap();
    let read = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut at = 16;
    let mut size = false;
    let mut data_types = false;
    while at < bytes.len() {
        let key = read(at);
        let length = read(at + 8) as usize;
        assert!(!matches!(key, 1 | 4));
        if key == 3 {
            assert_eq!(read(at + 32), 3840 * 2160 * 4);
            assert_eq!(read(at + 36), 3840 * 2160 * 4);
            size = true;
        }
        if key == 6 {
            assert_eq!(read(at + 32), (1 << 1) | (1 << 2));
            data_types = true;
        }
        at += 16 + ((length + 7) & !7);
    }
    assert!(size && data_types);
}
#[test]
fn batches_task_capture_linux_pipewire_resize_keeps_old_mapping_and_last_cpu_owner() {
    let _serial = fixture::SERIAL.lock().unwrap();
    let f = Fixture::new("pw-resize");
    let fd = fixture::fd(4096);
    unsafe {
        fixture::add(1, &fd, 3);
    }
    f.admit();
    let grant = f.image_grant(FrameSize::new(4, 3));
    let image = super::convert_rows_admitted(
        &[0u8; 48],
        16,
        super::super::VideoFormat::Bgra,
        FrameSize::new(4, 3),
        &grant,
    )
    .unwrap();
    f.memory
        .format(Some((
            super::super::VideoFormat::Bgra,
            FrameSize::new(8, 3),
        )))
        .unwrap();
    let second = fixture::fd(8192);
    unsafe {
        fixture::add(2, &second, 3);
    }
    let inspected = f.inspect();
    let before_grow = f
        .memory
        .authority()
        .approve_import(f.imports.need(inspected).unwrap())
        .unwrap();
    assert_eq!(unsafe { libc::ftruncate(second.as_raw_fd(), 12288) }, 0);
    let changed = unsafe { f.imports.map_approved(inspected, before_grow) }.unwrap_err();
    assert_eq!(changed.kind(), io::ErrorKind::Interrupted);
    assert_ne!(f.imports.pending().unwrap().id, inspected.pending.id);
    assert_eq!(fixture::observed().1, 1);
    f.admit();
    assert_eq!(fixture::observed().1, 2);
    assert_eq!(grant.live_bytes().unwrap(), 48);
    unsafe {
        libc::ftruncate(fd.as_raw_fd(), 32);
    }
    let block = SpaData {
        type_: 2,
        flags: 0,
        fd: fd.as_raw_fd() as i64,
        mapoffset: 0,
        maxsize: 48,
        data: std::ptr::null_mut(),
        chunk: std::ptr::null_mut(),
    };
    assert!(unsafe { f.imports.data_range(&block) }.is_err());
    drop(f);
    assert_eq!(grant.live_bytes().unwrap(), 48);
    drop(image);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_linux_pipewire_retirement_waits_for_native_views_and_callback_return() {
    let _serial = fixture::SERIAL.lock().unwrap();
    let f = Fixture::new("pw-retire");
    let fd = fixture::fd(4096);
    unsafe {
        fixture::add(1, &fd, 3);
    }
    f.admit();
    let live = f.native_live();
    fixture::views(1, 1);
    unsafe {
        fixture::remove(1);
        f.imports.reap(false);
    }
    assert_eq!(f.native_live(), live); // Whole-map handle is gone; native view remains.
    unsafe {
        fixture::release_views(1);
    }
    assert_eq!(f.native_live(), live); // removed callback returned, control has not reaped.
    unsafe {
        f.imports.reap(false);
    }
    assert_eq!(f.native_live(), 0);
    let silent = fixture::fd(4096);
    unsafe {
        fixture::add(1, &silent, 3 | (1 << 5));
    }
    f.admit();
    unsafe {
        fixture::remove(1);
        f.imports.reap(false);
    }
    assert!(f.native_live() > 0); // DONT_NOTIFY requires explicit Core completion.
}
#[test]
fn batches_task_capture_linux_pipewire_formats_pause_identity_and_three_actual_cpu_slots() {
    let _serial = fixture::SERIAL.lock().unwrap();
    let f = Fixture::new("pw-compat");
    let grant = f.image_grant(FrameSize::new(1, 1));
    let formats = [
        super::super::VideoFormat::Bgrx,
        super::super::VideoFormat::Rgbx,
        super::super::VideoFormat::Bgra,
        super::super::VideoFormat::Rgba,
    ];
    for format in formats {
        let image =
            super::convert_rows_admitted(&[1, 2, 3, 4], 4, format, FrameSize::new(1, 1), &grant)
                .unwrap();
        let expected = match format {
            super::super::VideoFormat::Bgrx | super::super::VideoFormat::Bgra => [1, 2, 3, 255],
            super::super::VideoFormat::Rgbx | super::super::VideoFormat::Rgba => [3, 2, 1, 255],
        };
        assert_eq!(image.image().data(), &expected);
    }
    let make = || {
        super::convert_rows_admitted(&[1, 2, 3, 4], 4, formats[0], FrameSize::new(1, 1), &grant)
            .unwrap()
    };
    let a = make();
    let b = make();
    let c = make();
    assert!(
        super::convert_rows_admitted(&[0; 4], 4, formats[0], FrameSize::new(1, 1), &grant).is_err()
    );
    f.latest.publish(a.clone());
    let held = match f.latest.take(true) {
        super::super::Taken::Frame(image) => image,
        _ => panic!("expected cached alias"),
    };
    assert!(held.shares_pixels(&a));
    f.latest.set_capture_active(false).unwrap();
    drop(a);
    drop(b);
    drop(c);
    assert_eq!(grant.live_bytes().unwrap(), 4);
    let waiting =
        std::sync::Mutex::new(crate::screen::os::portal_session::PortalState::WaitingInactive);
    let paused = crate::screen::os::portal_session::latest_for_state(&waiting);
    assert!(!paused.desired_active());
    for remote_desktop in [false, true] {
        let options = crate::screen::os::portal_session::PortalOptions {
            remote_desktop,
            ..Default::default()
        };
        assert_eq!(options.remote_desktop, remote_desktop);
    }
    assert!(buffers::check_target_override(Some(std::ffi::OsStr::new("foreign-node"))).is_err());
    let identity = super::super::identity::StreamIdentity::new(
        42,
        Some(77),
        "portal-session".into(),
        None,
        Some((960, 540)),
    );
    identity.on_state(1, 2);
    identity.on_state(3, 2);
    assert!(identity.belongs_to("portal-session"));
    identity.invalidate();
    identity.on_state(0, 3);
    assert!(identity.expired());
    drop(f);
    assert_eq!(grant.live_bytes().unwrap(), 4);
    drop(held);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
