//! A peer watch has its own stream lifetime; full consumers receive an overflow.
use std::{io,sync::{Arc,atomic::{AtomicBool,Ordering}},time::Duration};
use crate::vfs::{ChangeNotice,ChangeSubscription};
use super::{backend::PeerBackend,framing,fs_response::{FsWatchEvent,SILENT_SECS},io_deadline,peer_stream,
    wire::{Ctrl,FsRequest,FsResponse,FsWatch}};
struct Guard(Arc<AtomicBool>);
impl Drop for Guard { fn drop(&mut self) { self.0.store(true,Ordering::Relaxed); } }

pub(super) fn subscribe(backend:&PeerBackend,root:&str,tx:crossbeam_channel::Sender<ChangeNotice>)->io::Result<Option<ChangeSubscription>> {
    if !peer_stream::features(backend,root)?.watch_v1 { return Ok(None); }
    let cancel=Arc::new(AtomicBool::new(false)); let worker_cancel=cancel.clone();
    let source=backend.endpoint_source.clone(); let identity=backend.identity.clone(); let node=backend.node.clone();
    let lease=backend.mount_lease_token()?; let root=root.to_owned();
    std::thread::Builder::new().name("share-watch-client".into()).spawn(move || {
        let run=(|| {
            let endpoint=source.current()?;
            let mut opened=node.open_stream(&endpoint,&identity)?;
            let result=node.block_on(async {
                io_deadline::run("Share watch request",framing::send_ctrl(&mut opened.send,&Ctrl::Fs {
                    req:FsRequest::WatchExport(FsWatch { path:root }),lease })).await?;
                let mut generation=None; let mut ready=None; let mut overflow=false;
                loop {
                    flush(&tx,&mut ready,&mut overflow)?;
                    let (tag,bytes)={
                        let next=peer_stream::frame(&mut opened.recv,&worker_cancel,SILENT_SECS);
                        tokio::pin!(next);
                        let mut tick=tokio::time::interval(Duration::from_millis(100));
                        loop { tokio::select! {
                            result=&mut next=>break result?,
                            _=tick.tick()=>flush(&tx,&mut ready,&mut overflow)?,
                        } }
                    };
                    let FsResponse::Watch { event }=peer_stream::decode(tag,&bytes)? else { return Err(peer_stream::invalid("Unerwartete Watch-Meldung")); };
                    let current=match &event { FsWatchEvent::Ready { generation,.. } | FsWatchEvent::Changed { generation,.. }
                        | FsWatchEvent::Overflow { generation }=>Some(*generation),_=>None };
                    if current.zip(generation).is_some_and(|(current,last)| current<last) { return Err(peer_stream::invalid("Watch-Generation rückläufig")); }
                    if current.is_some() { generation=current; }
                    if let FsWatchEvent::Changed { paths,.. }=&event {
                        if paths.len()>super::fs_response::MAX_NOTICE_PATHS || paths.iter().any(|path| !path.is_empty()&&!peer_stream::relative(path)) {
                            return Err(peer_stream::invalid("Ungültiger Watch-Pfadhinweis"));
                        }
                    }
                    let terminal=matches!(event,FsWatchEvent::Unavailable { .. });
                    if let Some(notice)=event.into_notice() {
                        if matches!(notice,ChangeNotice::Ready { .. } | ChangeNotice::ReadyPartial { .. }) { ready=Some(notice); }
                        else {
                            match tx.try_send(notice) {
                                Ok(())=>{},Err(crossbeam_channel::TrySendError::Full(notice))=>{
                                    if terminal { return Err(io::Error::other(match notice { ChangeNotice::Ended(reason)=>reason,_=>"Watch beendet".into() })); }
                                    overflow=true;
                                }
                                Err(crossbeam_channel::TrySendError::Disconnected(_))=>return Err(io::ErrorKind::Interrupted.into()),
                            }
                        }
                    }
                    flush(&tx,&mut ready,&mut overflow)?;
                    if terminal { return Ok(()); }
                }
            });
            if let Err(error)=&result {
                peer_stream::abort(&mut opened.send,&mut opened.recv,worker_cancel.load(Ordering::Relaxed));
                if peer_stream::transport(error) { let _=node.invalidate_outgoing_session(&opened.session_key,opened.generation); }
            }
            result
        })();
        if let Err(error)=run {
            let mut notice=ChangeNotice::Ended(error.to_string());
            while !worker_cancel.load(Ordering::Relaxed) {
                match tx.send_timeout(notice,Duration::from_millis(100)) {
                    Ok(()) | Err(crossbeam_channel::SendTimeoutError::Disconnected(_))=>break,
                    Err(crossbeam_channel::SendTimeoutError::Timeout(pending))=>notice=pending,
                }
            }
        }
    })?;
    Ok(Some(ChangeSubscription::new(Guard(cancel))))
}
fn flush(tx:&crossbeam_channel::Sender<ChangeNotice>,ready:&mut Option<ChangeNotice>,overflow:&mut bool)->io::Result<()> {
    if let Some(notice)=ready.take() {
        match tx.try_send(notice) {
            Ok(())=>{},Err(crossbeam_channel::TrySendError::Full(notice))=>{ *ready=Some(notice); return Ok(()); },
            Err(crossbeam_channel::TrySendError::Disconnected(_))=>return Err(io::ErrorKind::Interrupted.into()),
        }
    }
    if *overflow {
        match tx.try_send(ChangeNotice::Overflow) {
            Ok(())=>*overflow=false,Err(crossbeam_channel::TrySendError::Full(_))=>{},
            Err(crossbeam_channel::TrySendError::Disconnected(_))=>return Err(io::ErrorKind::Interrupted.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_watch_backpressure_keeps_partial_ready_and_reports_overflow() {
        let (tx,rx)=crossbeam_channel::bounded(1);
        tx.send(ChangeNotice::Overflow).unwrap();
        let mut ready=Some(ChangeNotice::ReadyPartial { generation:Some(7) }); let mut overflow=true;
        flush(&tx,&mut ready,&mut overflow).unwrap(); assert!(ready.is_some());
        rx.recv().unwrap(); flush(&tx,&mut ready,&mut overflow).unwrap();
        assert_eq!(rx.recv().unwrap(),ChangeNotice::ReadyPartial { generation:Some(7) });
        flush(&tx,&mut ready,&mut overflow).unwrap(); assert_eq!(rx.recv().unwrap(),ChangeNotice::Overflow);
        let guard=Guard(Arc::new(AtomicBool::new(false))); let flag=guard.0.clone(); drop(guard); assert!(flag.load(Ordering::Relaxed));
    }
}
