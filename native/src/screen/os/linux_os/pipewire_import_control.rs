//! The only thread that calls the Parent authority. Native receive resume can
//! immediately reenter Core callbacks; their state mutex is unlocked then.
use super::*;
pub(in crate::screen::os::pipewire_stream) fn start(pointer:usize,loop_address:usize)->io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name("pipewire-admission".into()).spawn(move || {
        // SAFETY: PwStream joins before destroying the boxed callback data.
        let data=unsafe {&*(pointer as *const CallbackData)};
        let loop_pointer=loop_address as *mut c_void;let mut seen=0;
        while data.imports.open() && !data.identity.expired() {
            let inspected=unsafe {
                (data.api.thread_loop_lock)(loop_pointer);
                data.imports.reap(false);
                let result=data.imports.pending().map(|pending|data.imports.inspect(pending)).transpose();
                (data.api.thread_loop_unlock)(loop_pointer);result
            };
            let inspected=match inspected {Ok(value)=>value,Err(error)=>{data.fail(error.to_string());break;}};
            if let Some(inspected)=inspected {
                let need=match data.imports.need(inspected) {Ok(need)=>need,Err(error)=>{data.fail(error.to_string());break;}};
                let grant=match data.memory.authority().approve_import(need) {
                    Ok(grant)=>grant,Err(error)=>{if data.imports.open() {data.fail(error.to_string());}break;}
                };
                // Close may have cancelled a response which arrived concurrently.
                if !data.imports.open() || data.identity.expired() {let _=grant.retire();break;}
                let result=unsafe {
                    (data.api.thread_loop_lock)(loop_pointer);
                    let result=if data.imports.open() && !data.identity.expired() {
                        data.imports.map_approved(inspected,grant)
                    } else {let _=grant.retire();Ok(())};
                    // Resume may have removed mappings or delivered another
                    // add_mem. Reap after return; never reset that new pause.
                    data.imports.reap(false);
                    (data.api.thread_loop_unlock)(loop_pointer);result
                };
                if let Err(error)=result {
                    if error.kind()==io::ErrorKind::Interrupted {continue;}
                    data.fail(error.to_string());break;
                }
                continue;
            }
            match data.memory.frame_work(&mut seen) {
                Ok(Some((demand,grant)))=>unsafe {
                    (data.api.thread_loop_lock)(loop_pointer);
                    if !data.identity.expired() && data.memory.installed(&demand,grant) {data.latest.admitted_locked();}
                    (data.api.thread_loop_unlock)(loop_pointer);
                },
                Ok(None)=>data.memory.wait(),
                Err(error)=>{if data.imports.open() {data.fail(error.to_string());}break;}
            }
        }
    })
}

/// Builds the loop, context, core and stream and connects the stream.
///
/// # Safety
/// `data`, `events` and `hook` must stay at their addresses until the stream
/// is destroyed.
pub(in crate::screen::os::pipewire_stream) unsafe fn create(
    api: &Arc<PipewireApi>,
    fd: OwnedFd,
    name: &CString,
    data: &mut CallbackData,
    events: &mut PwStreamEvents,
    hook: &mut SpaHook,
) -> io::Result<(*mut c_void, *mut c_void, *mut c_void, *mut c_void)> {
    let fail = |what: &str| io::Error::other(format!("pipewire: {what} failed"));
    let thread_loop = (api.thread_loop_new)(name.as_ptr(), null());
    if thread_loop.is_null() {
        return Err(fail("pw_thread_loop_new"));
    }
    let context = (api.context_new)((api.thread_loop_get_loop)(thread_loop), null_mut(), 0);
    if context.is_null() {
        (api.thread_loop_destroy)(thread_loop);
        return Err(fail("pw_context_new"));
    }
    if (api.thread_loop_start)(thread_loop) < 0 {
        (api.context_destroy)(context);
        (api.thread_loop_destroy)(thread_loop);
        return Err(fail("pw_thread_loop_start"));
    }
    (api.thread_loop_lock)(thread_loop);
    // The core owns the descriptor from here on.
    let core = (api.context_connect_fd)(context, fd.into_raw_fd(), null_mut(), 0);
    let stream = if core.is_null() {
        null_mut()
    } else {
        match data.imports.attach(core) {
            Ok(())=>make_stream(api, core, name, data, events, hook),
            Err(error)=>{data.latest.fail(error.to_string());null_mut()}
        }
    };
    if stream.is_null() {
        if !core.is_null() {
            (api.core_disconnect)(core);data.imports.finish_core();
        }
        (api.context_destroy)(context);
        (api.thread_loop_unlock)(thread_loop);
        (api.thread_loop_stop)(thread_loop);
        (api.thread_loop_destroy)(thread_loop);
        let step = if core.is_null() {
            "pw_context_connect_fd"
        } else {
            "pw_stream_connect"
        };
        return Err(fail(step));
    }
    (api.thread_loop_unlock)(thread_loop);
    Ok((thread_loop, context, core, stream))
}

/// The stream on `core`, connected to `node_id`; null on failure.
///
/// # Safety
/// As `create`, with the thread loop locked.
unsafe fn make_stream(
    api: &PipewireApi,
    core: *mut c_void,
    name: &CString,
    data: &mut CallbackData,
    events: &mut PwStreamEvents,
    hook: &mut SpaHook,
) -> *mut c_void {
    let keys = [
        "media.type",
        "Video",
        "media.category",
        "Capture",
        "media.role",
        "Screen",
        "node.dont-fallback",
        "true",
        "node.dont-move",
        "true",
        "node.dont-reconnect",
        "true",
    ]
    .map(|text| CString::new(text).unwrap_or_default());
    let (node_id, serial) = data.identity.target();
    let target = CString::new(serial.as_deref().unwrap_or("")).unwrap_or_default();
    let target_key = CString::new("target.object").unwrap_or_default();
    let properties = (api.properties_new)(
        keys[0].as_ptr(),
        keys[1].as_ptr(),
        keys[2].as_ptr(),
        keys[3].as_ptr(),
        keys[4].as_ptr(),
        keys[5].as_ptr(),
        keys[6].as_ptr(), keys[7].as_ptr(),
        keys[8].as_ptr(), keys[9].as_ptr(),
        keys[10].as_ptr(), keys[11].as_ptr(),
        if serial.is_some() { target_key.as_ptr() } else { null() },
        target.as_ptr(),
        null::<c_char>(),
    );
    if properties.is_null() {
        return null_mut();
    }
    let stream = (api.stream_new)(core, name.as_ptr(), properties);
    if stream.is_null() {
        return null_mut();
    }
    data.stream.store(stream, Ordering::Release);
    let data_pointer: *mut CallbackData = data;
    (api.stream_add_listener)(stream, hook, events, data_pointer.cast());
    // The pod lives in 8-byte words, as PipeWire reads it.
    let bytes = enum_format_pod();
    let mut words = vec![0u64; bytes.len().div_ceil(8)];
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr().cast::<u8>(), bytes.len());
    let mut params: [*const SpaPod; 1] = [words.as_ptr().cast()];
    // Active negotiation must work before a producer publishes fixed buffers.
    // No MAP_BUFFERS, ALLOC_BUFFERS or RT_PROCESS: Core import gates mapping.
    let flags=connection_flags(data.latest.desired_active());
    let result = (api.stream_connect)(
        stream,
        PW_DIRECTION_INPUT,
        node_id,
        flags,
        params.as_mut_ptr(),
        1,
    );
    if result < 0 {
        (api.stream_destroy)(stream);
        return null_mut();
    }
    stream
}


pub(in crate::screen::os::pipewire_stream) fn connection_flags(active:bool)->u32 {
    PW_STREAM_FLAG_AUTOCONNECT | super::super::super::pipewire_abi::PW_STREAM_FLAG_DONT_RECONNECT
        | if active {0}else {super::super::super::pipewire_abi::PW_STREAM_FLAG_INACTIVE}
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::fixture_support::Fixture;
    use crate::screen::os::pipewire_stream::buffers::fixture;
    use crate::screen::core::capture_import::CaptureImportNeed;
    use crate::screen::core::capture_admission::{SourceKey,CaptureOwner};
    use crate::screen::core::host_codec_adapters::{HostContract,HostCodecPlan,HostOutputPlan};
#[test]
fn batches_task_capture_linux_pipewire_import_and_image_share_the_session_authority() {
    let _serial=fixture::SERIAL.lock().unwrap();let f=Fixture::new("pw-shared");
    let fd=fixture::fd(4096);unsafe {fixture::add(1,&fd,3);}f.admit();
    let image=f.image_grant(FrameSize::new(4,3));
    let connection=f.admission.import_connection(f.memory.source()).unwrap();
    let huge=CaptureImportNeed::new(f.memory.source().clone(),connection.id(1).unwrap(),1<<40,0).unwrap();
    assert!(f.memory.authority().approve_import(huge).is_err());
    assert!(f.native_live()>=4096);assert_eq!(image.plan().owner_limits()[CaptureOwner::SourcePixels as usize],144);
}
#[test]
fn batches_task_capture_linux_pipewire_unbound_import_has_no_image_right() {
    let _serial=fixture::SERIAL.lock().unwrap();let f=Fixture::new("pw-unbound");
    let fd=fixture::fd(4096);unsafe {fixture::add(1,&fd,3);}
    let inspected=f.inspect();let grant=f.memory.authority().approve_import(f.imports.need(inspected).unwrap()).unwrap();
    let wrong=SourceKey::new("pw-unbound","foreign").unwrap();
    assert!(grant.check_identity(&wrong,&inspected.pending.id).is_err());
    assert!(crate::screen::core::capture_bytes::AdmittedBytes::new(&grant.control_grant(),CaptureOwner::SourcePixels,4).is_err());
    let size=FrameSize::new(4,3);f.memory.format(Some((VideoFormat::Bgra,size))).unwrap();
    let output=HostOutputPlan::new(HostContract::Legacy,size,HostCodecPlan::Tiles).unwrap();
    assert!(f.memory.authority().approve(f.memory.capture_plan(&output).unwrap().unwrap()).is_err());
    unsafe {f.imports.map_approved(inspected,grant).unwrap();}
}
#[test]
fn batches_task_capture_linux_pipewire_close_late_grant_and_native_failures_never_map() {
    let _serial=fixture::SERIAL.lock().unwrap();
    {let f=Fixture::new("pw-late");let fd=fixture::fd(4096);unsafe {fixture::add(1,&fd,3);}
        let inspected=f.inspect();let grant=f.memory.authority().approve_import(f.imports.need(inspected).unwrap()).unwrap();
        f.imports.close();assert!(unsafe {f.imports.map_approved(inspected,grant)}.is_err());assert_eq!(fixture::observed().1,0);}
    {let f=Fixture::new("pw-pause-error");let fd=fixture::fd(4096);fixture::fail_pause();unsafe {fixture::add(1,&fd,3);}
        assert!(f.latest.check_failure().is_err());assert_eq!(fixture::observed().1,0);}
    {let f=Fixture::new("pw-map-error");let fd=fixture::fd(4096);unsafe {fixture::add(1,&fd,3);}
        let inspected=f.inspect();let grant=f.memory.authority().approve_import(f.imports.need(inspected).unwrap()).unwrap();
        fixture::fail_map();assert!(unsafe {f.imports.map_approved(inspected,grant)}.is_err());assert_eq!(fixture::observed().1,0);}
}
}
