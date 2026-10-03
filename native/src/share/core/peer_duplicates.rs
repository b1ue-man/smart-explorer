//! Host duplicate results are accepted only after complete group portions and summary.
use std::{collections::HashSet, io, sync::atomic::Ordering};
use crate::analytics::{DuplicateReport, ReclaimProgress};
use super::{backend::PeerBackend, fs_response::{FsDuplicateGroup, FsDuplicateMessage}, peer_stream,
    wire::{FsDuplicateSearch, FsRequest, FsResponse}};

pub(super) fn find(backend:&PeerBackend,root:&str,min_bytes:u64,p:&ReclaimProgress)->io::Result<Option<DuplicateReport>> {
    if p.cancel.load(Ordering::Relaxed) { return Err(io::ErrorKind::Interrupted.into()); }
    let features=peer_stream::features(backend,root)?;
    if !features.duplicate_search_v1 { return Ok(None); }
    let request=FsDuplicateSearch { path:root.into(),min_bytes,
        request_id:if features.analysis_reattach_v1 { Some(super::core::random_token(16).map_err(io::Error::other)?) } else { None } };
    for attempt in 0..2 {
        let result=receive(backend,request.clone(),p);
        match result {
            Err(error) if attempt==0 && features.analysis_reattach_v1 && peer_stream::transport(&error)=>{
                if p.cancel.load(Ordering::Relaxed) { return Err(io::ErrorKind::Interrupted.into()); }
                continue;
            }
            other=>return other.map(Some),
        }
    }
    Err(io::ErrorKind::NotConnected.into())
}
fn receive(backend:&PeerBackend,request:FsDuplicateSearch,p:&ReclaimProgress)->io::Result<DuplicateReport> {
    let root=format!("/{}",crate::share::fs::split_clean(&request.path)?.join("/"));
    let prefix=format!("{}/",root.trim_end_matches('/'));
    let mut groups=Vec::new(); let mut pending:Option<FsDuplicateGroup>=None;
    let mut names=HashSet::new(); let mut memory=0usize;
    peer_stream::call(backend,FsRequest::DuplicateSearch(request),&p.cancel,|response| {
        match response {
            FsResponse::Duplicates { message:FsDuplicateMessage::Progress { state } }=>{ state.apply(p); Ok(None) }
            FsResponse::Duplicates { message:FsDuplicateMessage::Groups { groups:portions } }=>{
                for portion in portions {
                    if portion.sha256.len()!=64 || !portion.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) || portion.files.is_empty() {
                        return Err(peer_stream::invalid("Ungültige SHA-256-Duplikatgruppe"));
                    }
                    for file in &portion.files {
                        crate::share::fs::split_clean(&file.path)?;
                        if !file.path.starts_with(&prefix) || !names.insert(file.path.clone()) {
                            return Err(peer_stream::invalid("Duplikatpfad außerhalb der Wurzel oder mehrfach vorhanden"));
                        }
                        memory=memory.saturating_add(file.path.capacity().saturating_mul(3)
                            +2*std::mem::size_of::<crate::analytics::ReclaimItem>()
                            +2*std::mem::size_of::<super::fs_response::FsDuplicateFile>()+128);
                    }
                    if memory>crate::transfer::memory_budget() { return Err(io::Error::new(io::ErrorKind::OutOfMemory,"Duplikatergebnis übersteigt den verfügbaren Empfängerspeicher")); }
                    let mut combined=match pending.take() {
                        Some(mut previous)=>{
                            if previous.sha256!=portion.sha256 || previous.size!=portion.size { return Err(peer_stream::invalid("Duplikatfortsetzung gehört zu anderer Gruppe")); }
                            previous.files.extend(portion.files); previous.more=portion.more; previous
                        }
                        None=>portion,
                    };
                    if combined.more { pending=Some(combined); }
                    else {
                        if combined.files.len()<2 { return Err(peer_stream::invalid("Duplikatgruppe enthält weniger als zwei Dateien")); }
                        combined.more=false; groups.push(combined.into_group());
                    }
                }
                Ok(None)
            }
            FsResponse::Duplicates { message:FsDuplicateMessage::Done { summary } }=>{
                if pending.is_some() || summary.groups!=groups.len() as u64 || summary.errors.len()>64
                    || summary.protected.len()>256*1024/std::mem::size_of::<crate::analytics::ProtectedOmission>() || summary.limits.len()>64 {
                    return Err(peer_stream::invalid("Duplikatsumme ist widersprüchlich oder Gruppe abgeschnitten"));
                }
                let diagnostic = summary.errors.iter().chain(&summary.limits).chain(summary.root_error.iter())
                    .map(|text| text.capacity().saturating_add(std::mem::size_of::<String>()*2)).fold(0usize,usize::saturating_add);
                let protected = summary.protected.iter().map(|area| area.area.capacity()
                    .saturating_add(std::mem::size_of::<crate::analytics::ProtectedOmission>()*2)).fold(0usize,usize::saturating_add);
                if memory.saturating_add(diagnostic).saturating_add(protected)>crate::transfer::memory_budget() {
                    return Err(io::Error::new(io::ErrorKind::OutOfMemory,"Duplikatdiagnosen überschreiten den Empfängerspeicher"));
                }
                let (summary,root_error)=summary.into_summary();
                p.files.store(summary.files,Ordering::Relaxed); p.bytes.store(summary.bytes,Ordering::Relaxed);
                Ok(Some(DuplicateReport { groups:std::mem::take(&mut groups),summary,root_error }))
            }
            _=>Err(peer_stream::invalid("Unerwartete Duplikat-Meldung")),
        }
    })
}
