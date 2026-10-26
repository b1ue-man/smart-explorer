//! Worker lifetime and permanently active resource-control dispatcher.
use super::*;
pub(super) fn run()->i32 {
    if let Err(error)=super::super::worker_lifetime::arm_worker() {eprintln!("screen host process: {error}");return 1;}
    let _stop=match crate::daemon::spawn_update_stop_watch(crate::daemon::UPDATE_STOP_POLL,||std::process::exit(crate::daemon::EXIT_UPDATE_STOP)) {
        Ok(watch)=>watch,Err(error)=>{eprintln!("screen host process update-stop: {error}");return 1;}
    };
    let output:dispatch::Output=Arc::new(std::sync::Mutex::new(Box::new(io::stdout())));
    let remote=dispatch::RemoteAuthority::new(output.clone());
    let requests=match dispatch::start(io::stdin(),remote.clone()) {Ok(requests)=>requests,Err(error)=>{eprintln!("screen host process: {error}");return 1;}};
    let mut worker=Worker {admitted:None,backend:super::super::platform_backend(),source:None,encoder:None,
        refused:Vec::new(),produced:0,checked:Instant::now(),control:false,capture_active:true,reference_generation:0,
        family:None,prepared:None,planned:None,capture_session:None,remote:remote.clone(),release_sink:None};
    for request in requests {
        let request=match request {Ok(WorkerRequest::Close)=>break,Ok(request)=>request,
            Err(error)=>{eprintln!("screen host process: {error}");return 1;}};
        let reply=worker.handle(request);
        let generation=worker.source.as_ref().map_or(0,|source|source.reference_generation());
        if generation!=worker.reference_generation {worker.reference_generation=generation;worker.encoder=None;}
        let regions=worker.source.as_ref().is_none_or(|source|source.supports_frame_regions());
        let limits=worker.source.as_ref().map_or(FrameLimits::LEGACY,|source|source.frame_limits());
        let family_layout_revision=worker.source.as_ref().and_then(|source|source.family_layout_revision());
        let reply=if matches!(&reply,WorkerReply::CaptureControl(_)) {reply}
            else {WorkerReply::Reference {generation,regions,limits,family_layout_revision,reply:Box::new(reply)}};
        let sent=(|| {
            let grant=crate::screen::core::capture_transport::picture_grant(&reply)?;
            let mut output=output.lock().map_err(|_|io::Error::other("worker output poisoned"))?;
            if let Some(grant)=grant {
                let id=remote.id_for(&grant)?;
                crate::screen::core::capture_transport::send_owned(&mut *output,&reply,id,&grant)
            } else {send_reply(&mut *output,&reply)}
        })();
        if let Err(error)=sent {eprintln!("screen host process: {error}");return 1;}
    }
    // Worker/source/encoder teardown runs before process exit; the Parent still
    // retains unretired grants until its wait confirms the actual exit.
    drop(worker);0
}
