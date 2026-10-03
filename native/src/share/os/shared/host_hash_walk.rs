//! Host hash walks keep omissions explicit and local traversal handle-confined.
use std::{io::{self, Read}, path::Path, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use crate::{analytics::Progress, local_access::{DirectoryHandle, EntryKind}, share::{fs_access::FsAccess,
    fs_response::{FsHashEntry, FsHashWalkMessage, FsOmission, FsOmissionReason, LIST_BATCH_MAX_BYTES}, host_list, host_stream,
    wire::{FsHashAlgo, FsHashWalk, FsResponse}}, vfs::Scheme};

struct Walk<'a> {
    request: &'a FsHashWalk, cancel: &'a AtomicBool, tx: &'a mpsc::Sender<io::Result<FsResponse>>,
    progress: &'a Progress, entries: Vec<FsHashEntry>, omitted: Vec<FsOmission>, bytes: usize, totals: (u64,u64),
}
impl Walk<'_> {
    fn check(&self) -> io::Result<()> { if self.cancel.load(Ordering::Relaxed) { Err(io::ErrorKind::Interrupted.into()) } else { Ok(()) } }
    fn add(&mut self, entry: Result<FsHashEntry, FsOmission>) -> io::Result<()> {
        self.check()?;
        let cost = match &entry { Ok(entry) => entry.wire_bytes(), Err(hole) => hole.wire_bytes() }.saturating_mul(6);
        if cost > LIST_BATCH_MAX_BYTES { return Err(io::Error::new(io::ErrorKind::InvalidData, "Prüfsummenpfad überschreitet das Drahtformat")); }
        if self.bytes + cost > LIST_BATCH_MAX_BYTES { self.flush()?; }
        self.bytes += cost;
        match entry { Ok(entry) => { self.entries.push(entry); self.totals.0 += 1; },
            Err(hole) => { self.omitted.push(hole); self.totals.1 += 1; } }
        Ok(())
    }
    fn hole(&mut self, rel: &str, reason: FsOmissionReason, detail: impl Into<String>) -> io::Result<()> {
        self.add(Err(FsOmission { rel: rel.into(), reason, detail: detail.into() }))
    }
    fn failed(&mut self, rel: &str, error: io::Error) -> io::Result<()> {
        let reason = if error.kind() == io::ErrorKind::NotFound { FsOmissionReason::Vanished } else { FsOmissionReason::Unreadable };
        self.hole(rel, reason, error.to_string())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.entries.is_empty() && self.omitted.is_empty() { return Ok(()); }
        host_stream::emit(self.tx, self.cancel, FsResponse::HashWalk { message: FsHashWalkMessage::Batch {
            entries: std::mem::take(&mut self.entries), omitted: std::mem::take(&mut self.omitted) } })?;
        self.bytes = 0; Ok(())
    }
    fn digest(&self, mut read: impl Read, expected: u64) -> io::Result<Option<String>> {
        if self.request.algo.is_none() { return Ok(None); }
        let mut sha = Sha256::new(); let mut md5 = md5::Context::new();
        let mut buffer = vec![0; 1024 * 1024]; let mut size = 0u64;
        loop {
            self.check()?;
            let n = match read.read(&mut buffer) { Err(e) if e.kind() == io::ErrorKind::Interrupted => continue, result => result? };
            if n == 0 { break; } size = size.saturating_add(n as u64);
            if size > expected { break; }
            match self.request.algo { Some(FsHashAlgo::Sha256) => sha.update(&buffer[..n]),
                Some(FsHashAlgo::Md5) => md5.consume(&buffer[..n]), _ => {} }
            self.progress.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
        if size != expected { return Err(io::Error::new(io::ErrorKind::InvalidData, "Dateigröße hat sich beim Lesen geändert")); }
        Ok(Some(match self.request.algo { Some(FsHashAlgo::Sha256) => format!("{:x}",sha.finalize()),
            Some(FsHashAlgo::Md5) => format!("{:x}",md5.compute()), _ => return Ok(None) }))
    }
    fn local(&mut self, directory: &DirectoryHandle, physical: &Path, rel: &str, depth: usize, excluded: &[std::path::PathBuf]) -> io::Result<()> {
        self.check()?;
        if depth > 2048 { return self.hole(rel, FsOmissionReason::Unreadable, "Verzeichnistiefe überschreitet den sicheren Walk-Stack"); }
        let entries = match directory.read_directory() { Ok(entries) => entries, Err(e) => return self.failed(rel,e) };
        for entry in entries {
            self.check()?;
            let entry = match entry { Ok(entry) => entry, Err(error) => { self.failed(rel,error)?; continue; } };
            let name = entry.name.to_string_lossy();
            if host_list::hidden(&name) || excluded.iter().any(|p| p == &physical.join(&entry.name)) { continue; }
            let child = if rel.is_empty() { name.into_owned() } else { format!("{rel}/{name}") };
            if entry.unreachable || entry.name.to_str().is_none() || crate::vfs::validate_child_name(&entry.name.to_string_lossy()).is_err() {
                self.hole(&child, FsOmissionReason::Unrepresentable, "Name ist kein adressierbarer Share-Pfad")?; continue;
            }
            if entry.is_link_like || entry.kind == EntryKind::Link { self.hole(&child, FsOmissionReason::Link, "Link-Grenze")?; continue; }
            if entry.kind == EntryKind::Other { self.hole(&child, FsOmissionReason::Special, "Kein regulärer Datenstrom")?; continue; }
            if entry.kind == EntryKind::Directory {
                match directory.open_child(&entry.name) {
                    Ok(handle) => {
                        if let Err(error) = crate::share::fs::ensure_local_share_handle_allowed(&handle) {
                            self.failed(&child, error)?;
                            continue;
                        }
                        self.add(Ok(FsHashEntry { rel: child.clone(), is_dir: true, size: 0, mtime_ms: entry.mtime_ms, digest: None }))?;
                        self.local(&handle, &physical.join(&entry.name), &child, depth+1, excluded)?;
                    }
                    Err(error) => self.failed(&child,error)?,
                }
            } else if entry.size >= self.request.min_bytes {
                let result = (|| {
                    let file = directory.open_regular_child(&entry.name)?;
                    let before = file.metadata()?;
                    if before.len() != entry.size { return Err(io::Error::new(io::ErrorKind::InvalidData, "Datei nach der Auflistung geändert")); }
                    let digest = self.digest(&file, entry.size)?;
                    let after = file.metadata()?;
                    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "Datei beim Hashen geändert"));
                    }
                    Ok(FsHashEntry { rel: child.clone(), is_dir: false, size: entry.size, mtime_ms: entry.mtime_ms, digest })
                })();
                match result { Ok(entry) => { self.progress.files.fetch_add(1, Ordering::Relaxed); self.add(Ok(entry))?; }, Err(error) => self.failed(&child,error)? }
            }
        }
        Ok(())
    }
    fn remote(&mut self, backend: &dyn crate::vfs::Backend, path: &str, rel: &str, depth: usize) -> io::Result<()> {
        self.check()?;
        if depth > 2048 { return self.hole(rel, FsOmissionReason::Unreadable, "Verzeichnistiefe überschreitet den sicheren Walk-Stack"); }
        let listing = match crate::vfs::list_dir_tolerant(backend,path) { Ok(listing) => listing, Err(error) => return self.failed(rel,error) };
        let relative = |name: &str| -> String { if rel.is_empty() { name.into() } else { format!("{rel}/{name}") } };
        for hole in listing.omitted { self.hole(&relative(&hole.rel), hole.reason.into(), hole.detail)?; }
        for entry in listing.entries {
            self.check()?;
            if host_list::hidden(&entry.name) { continue; }
            let child = relative(&entry.name);
            if crate::vfs::validate_child_name(&entry.name).is_err() { self.hole(&child,FsOmissionReason::Unrepresentable,"Name ist kein Share-Pfad")?; continue; }
            if entry.is_symlink { self.hole(&child,FsOmissionReason::Link,"Link-Grenze")?; continue; }
            if entry.special { self.hole(&child,FsOmissionReason::Special,"Kein regulärer Datenstrom")?; continue; }
            let full = match crate::vfs::sync_child_path(backend, path, &entry.name) {
                Ok(full) => full,
                Err(error) => { self.failed(&child, error)?; continue; }
            };
            if entry.is_dir {
                self.add(Ok(FsHashEntry { rel: child.clone(), is_dir:true,size:0,mtime_ms:entry.mtime_ms,digest:None }))?;
                self.remote(backend,&full,&child,depth+1)?;
            } else if entry.size >= self.request.min_bytes {
                let result = crate::vfs::open_read_regular(backend,&full,entry.id.as_deref()).and_then(|read| self.digest(read,entry.size));
                match result { Ok(digest) => { self.progress.files.fetch_add(1,Ordering::Relaxed); self.add(Ok(FsHashEntry {
                    rel:child,is_dir:false,size:entry.size,mtime_ms:entry.mtime_ms,digest }))?; }, Err(e) => self.failed(&child,e)? }
            }
        }
        Ok(())
    }
}

pub(in crate::share) fn run(request: FsHashWalk, access: FsAccess, tx: mpsc::Sender<io::Result<FsResponse>>, cancel: Arc<AtomicBool>, p: Progress) -> io::Result<()> {
    if request.algo == Some(FsHashAlgo::Unknown) { return Err(io::Error::new(io::ErrorKind::Unsupported,"Hashalgorithmus nicht unterstützt")); }
    let mut walk = Walk { request:&request,cancel:&cancel,tx:&tx,progress:&p,entries:Vec::new(),omitted:Vec::new(),bytes:0,totals:(0,0) };
    visit(&mut walk, &request.path, &access, "")?;
    walk.flush()?;
    host_stream::emit(&tx,&cancel,FsResponse::HashWalk { message:FsHashWalkMessage::Done { entries:walk.totals.0,omitted:walk.totals.1 } })
}
fn visit(walk: &mut Walk<'_>, path: &str, access: &FsAccess, rel: &str) -> io::Result<()> {
    let parts = crate::share::fs::split_clean(path)?;
    if access.is_dynamic() && (parts.is_empty() || parts == ["Verbindungen"]) {
        for entry in access.list_dir(path)? {
            crate::vfs::validate_child_name(&entry.name)?;
            let child = if rel.is_empty() { entry.name.clone() } else { format!("{rel}/{}",entry.name) };
            walk.add(Ok(FsHashEntry { rel:child.clone(),is_dir:true,size:0,mtime_ms:0,digest:None }))?;
            visit(walk,&format!("{}/{}",path.trim_end_matches('/'),entry.name),access,&child)?;
        }
        return Ok(());
    }
    let target = match access.resolve(path) { Ok(target) => target, Err(error) => return walk.failed(rel,error) };
    if target.backend.scheme() == Scheme::Local {
        let physical = crate::share::storage_roots::local_path(&target)?;
        if crate::share::storage_roots::is_private(&physical) {
            return walk.hole(rel,FsOmissionReason::Unreadable,"Interner Host-Speicher ist geschützt");
        }
        match crate::share::storage_roots::open_local(&target) { Ok(handle) => walk.local(&handle,&physical,rel,0,&crate::share::storage_roots::excluded()), Err(e) => walk.failed(rel,e) }
    } else { walk.remote(&*target.backend,&target.path,rel,0) }
}
