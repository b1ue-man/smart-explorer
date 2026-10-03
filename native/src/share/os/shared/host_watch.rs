//! Watch sessions retain native handles and preserve overflow and partial coverage.
use std::{collections::{HashMap, HashSet}, io, time::Duration};
use iroh::endpoint::SendStream;
use crate::{analytics::Progress, share::{framing, fs_access::FsAccess, fs_response::FsWatchEvent,
    host_list, io_deadline, session::PeerPrincipal, storage_roots, wire::{FsResponse, FsWatch}}, watch};

struct Session {
    _roots: storage_roots::Roots,
    _hold: crate::share::analysis_resources::Reservation,
    _pins: Vec<crate::local_access::DirectoryHandle>,
    local: Vec<watch::WatchHandle>, paths: HashMap<watch::WatchId,String>,
    remote: Vec<crate::vfs::ChangeSubscription>,
    remote_rx: Vec<(String,crossbeam_channel::Receiver<crate::vfs::ChangeNotice>)>,
    rx: crossbeam_channel::Receiver<watch::WatchMessage>,
}

pub(in crate::share) async fn serve(mut send: SendStream, request: FsWatch, access: FsAccess, principal: PeerPrincipal) -> io::Result<()> {
    let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
    let p = Progress::default();
    access.register_cancel(&p.cancel)?;
    let authority = access.clone();
    let _stop = crate::share::host_stream::Stop(p.cancel.clone());
    let hold = crate::share::analysis_resources::Reservation::metadata(4096 + request.path.capacity() as u64)?;
    let stopped = send.stopped(); tokio::pin!(stopped);
    let ticket = crate::share::analysis_admission::watch_setup().enqueue(principal);
    let acquire = ticket.acquire(); tokio::pin!(acquire);
    let mut alive = tokio::time::interval(Duration::from_secs(crate::share::fs_response::ALIVE_SECS));
    let permit = loop { tokio::select! {
        _ = &mut stopped => return Ok(()),
        permit = &mut acquire => break permit,
        _ = alive.tick() => { notify(&mut send,&authority,FsWatchEvent::Alive).await?; },
    } };
    let (result, mut receive) = tokio::sync::oneshot::channel();
    let worker_progress = p.clone();
    std::thread::Builder::new().name("share-watch-setup".into()).spawn(move || {
        let _permit = permit;
        let prepared = setup(request, access, worker_progress, hold);
        let _ = result.send(prepared);
    })?;
    let session = loop { tokio::select! {
        _ = &mut stopped => return Ok(()),
        result = &mut receive => match result.map_err(io::Error::other)? {
            Ok(session) => break session,
            Err(error) => return unavailable(&mut send,&authority,error.to_string()).await,
        },
        _ = alive.tick() => { notify(&mut send,&authority,FsWatchEvent::Alive).await?; },
    } };
    let Session { _roots, _hold, _pins, local, paths, remote, remote_rx, rx } = session;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut generation = 0u64; let mut ready = HashSet::new(); let mut remote_ready = HashSet::new();
    let mut complete = true; let mut announced = false;
    // Empty synthetic exports are live too; policy revocation closes the session.
    if local.is_empty() && remote.is_empty() { notify(&mut send,&authority,FsWatchEvent::Ready { generation,complete:true }).await?; announced=true; }
    loop { tokio::select! {
        _ = &mut stopped => return Ok(()),
        _ = alive.tick() => { notify(&mut send,&authority,FsWatchEvent::Alive).await?; },
        _ = tick.tick() => {
            authority.check_read()?;
            p.check_cancel()?;
            let mut changed = Vec::new(); let mut overflow = false;
            for message in rx.try_iter() {
                let prefix = paths.get(&message.id).map_or("",String::as_str);
                match message.event {
                    watch::WatchEvent::Ready(coverage) => {
                        ready.insert(message.id); complete &= coverage == watch::Coverage::Complete;
                    }
                    watch::WatchEvent::Change(change) => changed.push(relative(prefix,&change.rel)),
                    watch::WatchEvent::Overflow => overflow = true,
                    watch::WatchEvent::Unavailable(reason) => return unavailable(&mut send,&authority,format!("{reason:?}")).await,
                }
            }
            for (index,(prefix,receiver)) in remote_rx.iter().enumerate() {
                for notice in receiver.try_iter() {
                    match notice {
                        crate::vfs::ChangeNotice::Ready { .. } => { remote_ready.insert(index); }
                        crate::vfs::ChangeNotice::ReadyPartial { .. } => { remote_ready.insert(index); complete=false; }
                        crate::vfs::ChangeNotice::Changed { paths,.. } => {
                            if paths.is_empty() { overflow=true; } else { changed.extend(paths.iter().map(|p| relative(prefix,p))); }
                        }
                        crate::vfs::ChangeNotice::Overflow => overflow=true,
                        crate::vfs::ChangeNotice::Ended(reason) => return unavailable(&mut send,&authority,reason).await,
                    }
                }
            }
            if !announced && ready.len()==local.len() && remote_ready.len()==remote.len() {
                notify(&mut send,&authority,FsWatchEvent::Ready { generation,complete }).await?; announced=true;
            }
            if overflow || !changed.is_empty() {
                generation = generation.checked_add(1).ok_or_else(|| io::Error::other("Watch-Generation erschöpft"))?;
                let event = if overflow { FsWatchEvent::Overflow { generation } } else {
                    changed.sort(); changed.dedup();
                    if changed.len()>crate::share::fs_response::MAX_NOTICE_PATHS { changed.clear(); }
                    FsWatchEvent::Changed { generation,paths:changed }
                };
                notify(&mut send,&authority,event).await?;
            }
        }
    } }
}

fn setup(request: FsWatch, access: FsAccess, p: Progress,
    mut hold: crate::share::analysis_resources::Reservation,
) -> io::Result<Session> {
    let roots = storage_roots::resolve(&request.path,&access,&p)?;
    let mut failures = Vec::new(); roots.plan.failures(&mut failures);
    if let Some(failure) = failures.first() { return Err(io::Error::other(failure.clone())); }
    hold.add(roots.roots.len() as u64 * 4096)?;
    let (tx, rx) = crossbeam_channel::bounded(64);
    let mut local = Vec::new(); let mut paths = HashMap::new();
    let mut remote = Vec::new(); let mut remote_rx = Vec::new();
    let mut pins = Vec::new();
    let root = format!("/{}",crate::share::fs::split_clean(&request.path)?.join("/"));
    for target in &roots.roots {
        p.check_cancel()?;
        let prefix = target.visible.strip_prefix(root.trim_end_matches('/')).unwrap_or_default().trim_start_matches('/').to_owned();
        if let Some(physical) = &target.physical {
            let pin = storage_roots::open_local(&target.target)?;
            let filter = watch::WatchFilter::new(|entry| !entry.rel.split('/').any(host_list::hidden));
            match watch::watch_confined(&pin,physical,watch::WatchOptions { cross_mounts:true },filter,watch::WatchSink::Channel(tx.clone())) {
                Ok(handle) => { paths.insert(handle.id(),prefix); local.push(handle); pins.push(pin); }
                Err(error) => return Err(error),
            }
        } else {
            let (sender,receiver) = crossbeam_channel::bounded(64);
            match crate::vfs::change_signal(&*target.target.backend,&target.target.path,Duration::from_secs(30),sender) {
                Ok(Some(subscription)) => { remote.push(subscription); remote_rx.push((prefix,receiver)); }
                Ok(None) => return Err(io::Error::new(io::ErrorKind::Unsupported,format!("{} kann keine Änderungen melden",target.visible))),
                Err(error) => return Err(error),
            }
        }
    }
    drop(tx);
    Ok(Session { _roots:roots, _hold:hold, _pins:pins, local, paths, remote, remote_rx, rx })
}
fn relative(prefix: &str, rel: &str) -> String {
    match (prefix.is_empty(),rel.is_empty()) { (true,_) => rel.into(), (_,true) => prefix.into(), _ => format!("{prefix}/{rel}") }
}
async fn unavailable(send: &mut SendStream,access: &FsAccess,reason:String) -> io::Result<()> {
    notify(send,access,FsWatchEvent::Unavailable { reason }).await?; send.finish().map_err(io::Error::other)
}
async fn notify(send: &mut SendStream,access: &FsAccess,event:FsWatchEvent) -> io::Result<()> {
    access.check_read()?;
    io_deadline::run("Share watch notice",framing::reply(send,FsResponse::Watch { event })).await
}
