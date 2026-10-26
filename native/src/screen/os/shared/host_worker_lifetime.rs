//! A failed kill/wait keeps the actual child and all unretired credit alive.
use std::{process::Child,sync::{Arc,Mutex},time::Duration};
use super::capture_broker::CaptureBroker;
pub(super) fn retire(child:Child,broker:Arc<CaptureBroker>) {
    let retiring=Arc::new(Mutex::new(Some((child,broker))));
    let watcher=retiring.clone();
    let started=std::thread::Builder::new().name("screen-host-retiring".into()).spawn(move|| {
        loop {
            let mut state=watcher.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some((child,_))=state.as_mut() else {return;};
            if matches!(child.try_wait(),Ok(Some(_))) {
                if let Some((_,broker))=state.take() {broker.process_exited();}
                return;
            }
            let _=child.kill();drop(state);std::thread::sleep(Duration::from_secs(1));
        }
    });
    if let Err(error)=started {
        eprintln!("screen host retirement watcher could not start; child credit retained: {error}");
        // No exit proof exists. Retain this one owner rather than inventing
        // success and making the still-running native resources unaccounted.
        std::mem::forget(retiring);
    }
}
