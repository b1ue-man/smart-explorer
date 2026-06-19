use crate::vfs::Backend;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::paths::{join, parent_of};
use super::persistence::versions_dir;
use super::types::{Action, BisyncOptions, BisyncStats, Direction, Sig, Throttle};

/// Insert a "(Konflikt <timestamp>)" tag before the extension of a relative path.
fn conflict_name(rel: &str) -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    match rel.rfind('.') {
        // only treat as an extension if the dot is in the final path segment
        Some(i) if i > rel.rfind('/').map(|s| s + 1).unwrap_or(0) => {
            format!("{} (Konflikt {}){}", &rel[..i], ts, &rel[i..])
        }
        _ => format!("{} (Konflikt {})", rel, ts),
    }
}

/// Delete a file, optionally to the OS Recycle Bin (local paths only). For a
/// remote path (or if trashing fails) it falls back to the backend's hard delete.
fn delete_file(be: &dyn Backend, path: &str, use_recycle: bool) -> io::Result<()> {
    if use_recycle && !path.contains("://") && std::path::Path::new(path).exists() {
        if trash::delete(path).is_ok() {
            return Ok(());
        }
    }
    be.remove_file(path)
}

/// Stream-copy one file between backends, creating the destination parent.
/// When `atomic`, writes to a temp sibling then renames into place (safe copies);
/// `throttle` rate-limits the transfer across all workers.
fn copy_between(
    src: &dyn Backend,
    sp: &str,
    dst: &dyn Backend,
    dp: &str,
    atomic: bool,
    throttle: &Throttle,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    use std::io::{Read, Write};
    // Safe-copies (temp then rename) are only correct where rename atomically
    // REPLACES the destination. On backends like Google Drive a rename creates a
    // duplicate same-named file instead of overwriting, so write in place there.
    let atomic = atomic && dst.rename_overwrites();
    if let Some(parent) = parent_of(dp) {
        let _ = dst.mkdir_all(&parent);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let write_path = if atomic {
        format!("{}.se-tmp-{:x}", dp, nanos)
    } else {
        dp.to_string()
    };
    let mut r = src.open_read(sp)?;
    let mut w = dst.open_write(&write_path)?;
    let mut buf = vec![0u8; 1 << 18];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(w);
            if atomic {
                let _ = dst.remove_file(&write_path);
            }
            return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
        }
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                if atomic {
                    let _ = dst.remove_file(&write_path);
                }
                return Err(e);
            }
        };
        if let Err(e) = w.write_all(&buf[..n]) {
            if atomic {
                let _ = dst.remove_file(&write_path);
            }
            return Err(e);
        }
        total += n as u64;
        throttle.consume(n as u64);
    }
    w.flush()?;
    drop(w);
    if atomic {
        dst.rename(&write_path, dp)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    }
    Ok(total)
}

/// Reversible backup: copy `path` (on `be`) into the local versions store before
/// it is overwritten/deleted. Best-effort; failure doesn't abort the sync but is
/// reported by the caller via the returned error.
fn back_up(be: &dyn Backend, path: &str, rel: &str, versions_dir: &PathBuf) -> io::Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = versions_dir.join(ts.to_string()).join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut r = be.open_read(path)?;
    let mut f = std::fs::File::create(&dest)?;
    io::copy(&mut r, &mut f)?;
    Ok(())
}

/// Execute one planned action (copy with reversible backup, or delete),
/// returning its contribution to the run stats. Network-bound and side-effect
/// free w.r.t. shared state, so many run concurrently in `apply`.
/// Re-stat the destination and confirm its size matches the bytes written.
fn verify_copy(dst: &dyn Backend, dp: &str, expected: u64) -> io::Result<()> {
    let got = dst.stat(dp).map(|m| m.size).unwrap_or(u64::MAX);
    if got != expected {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Überprüfung fehlgeschlagen: {} ≠ {} Bytes", got, expected),
        ));
    }
    Ok(())
}

fn run_one(
    act: &Action,
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &PathBuf,
    throttle: &Throttle,
    cancel: &AtomicBool,
) -> io::Result<BisyncStats> {
    let mut st = BisyncStats::default();
    match act {
        Action::CopyAtoB(rel) => {
            let dp = join(root_b, rel);
            if opts.reversible && b.exists(&dp) {
                let _ = back_up(b, &dp, rel, versions_dir);
            }
            let n = copy_between(a, &join(root_a, rel), b, &dp, opts.atomic, throttle, cancel)?;
            if opts.verify {
                verify_copy(b, &dp, n)?;
            }
            st.bytes += n;
            st.a_to_b += 1;
            // Move (one-way): remove the source after a successful copy.
            if opts.move_files && opts.direction != Direction::Both {
                let sp = join(root_a, rel);
                if opts.reversible {
                    let _ = back_up(a, &sp, rel, versions_dir);
                }
                if a.remove_file(&sp).is_ok() {
                    st.deleted += 1;
                }
            }
        }
        Action::CopyBtoA(rel) => {
            let dp = join(root_a, rel);
            if opts.reversible && a.exists(&dp) {
                let _ = back_up(a, &dp, rel, versions_dir);
            }
            let n = copy_between(b, &join(root_b, rel), a, &dp, opts.atomic, throttle, cancel)?;
            if opts.verify {
                verify_copy(a, &dp, n)?;
            }
            st.bytes += n;
            st.b_to_a += 1;
            if opts.move_files && opts.direction != Direction::Both {
                let sp = join(root_b, rel);
                if opts.reversible {
                    let _ = back_up(b, &sp, rel, versions_dir);
                }
                if b.remove_file(&sp).is_ok() {
                    st.deleted += 1;
                }
            }
        }
        Action::DeleteB(rel) => {
            let p = join(root_b, rel);
            if opts.reversible {
                let _ = back_up(b, &p, rel, versions_dir);
            }
            delete_file(b, &p, opts.use_recycle)?;
            st.deleted += 1;
        }
        Action::DeleteA(rel) => {
            let p = join(root_a, rel);
            if opts.reversible {
                let _ = back_up(a, &p, rel, versions_dir);
            }
            delete_file(a, &p, opts.use_recycle)?;
            st.deleted += 1;
        }
        Action::KeepBothAtoB(rel) => {
            let bp = join(root_b, rel);
            // Preserve B's losing version as a conflict copy that will sync back.
            if b.exists(&bp) {
                let cp = join(root_b, &conflict_name(rel));
                let _ = copy_between(b, &bp, b, &cp, opts.atomic, throttle, cancel);
            }
            st.bytes += copy_between(a, &join(root_a, rel), b, &bp, opts.atomic, throttle, cancel)?;
            st.a_to_b += 1;
        }
        Action::KeepBothBtoA(rel) => {
            let ap = join(root_a, rel);
            if a.exists(&ap) {
                let cp = join(root_a, &conflict_name(rel));
                let _ = copy_between(a, &ap, a, &cp, opts.atomic, throttle, cancel);
            }
            st.bytes += copy_between(b, &join(root_b, rel), a, &ap, opts.atomic, throttle, cancel)?;
            st.b_to_a += 1;
        }
    }
    Ok(st)
}

/// Apply the planned actions, with reversible backups. Returns stats; errors are
/// counted (and the rel/message collected) rather than aborting.
///
/// Transfers run **concurrently** up to `min(a, b).parallelism()` — the slower
/// side caps it, so SFTP/FTP (which report 1) stay serial while local↔Drive
/// runs many files at once. This is the headline fix for the "27k small files
/// at 0.1 Mbit/s" case: those transfers are latency-bound, not bandwidth-bound.
/// Destination folders are created lazily by `copy_between`; the backends'
/// `mkdir_all` is concurrency-safe (Drive serializes folder creation).
pub fn apply(
    actions: &[Action],
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &PathBuf,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> BisyncStats {
    if opts.dry_run {
        let mut st = BisyncStats::default();
        for act in actions {
            match act {
                Action::CopyAtoB(_) | Action::KeepBothAtoB(_) => st.a_to_b += 1,
                Action::CopyBtoA(_) | Action::KeepBothBtoA(_) => st.b_to_a += 1,
                Action::DeleteA(_) | Action::DeleteB(_) => st.deleted += 1,
            }
        }
        return st;
    }

    let mut par = a
        .parallelism()
        .min(b.parallelism())
        .max(1)
        .min(actions.len().max(1));
    if opts.max_transfers > 0 {
        par = par.min(opts.max_transfers);
    }

    let throttle = Throttle::new(opts.bwlimit_bps);
    let merged: Mutex<(BisyncStats, Vec<(String, String)>)> =
        Mutex::new((BisyncStats::default(), Vec::new()));
    let idx = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..par {
            scope.spawn(|| {
                let mut local = BisyncStats::default();
                let mut local_errs: Vec<(String, String)> = Vec::new();
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i >= actions.len() {
                        break;
                    }
                    let act = &actions[i];
                    // Retry transient failures with a delay.
                    let mut attempt = 0u32;
                    let res = loop {
                        match run_one(act, a, root_a, b, root_b, opts, versions_dir, &throttle, cancel) {
                            Ok(s) => break Ok(s),
                            Err(e) => {
                                if attempt >= opts.retries || cancel.load(Ordering::Relaxed) {
                                    break Err(e);
                                }
                                attempt += 1;
                                std::thread::sleep(std::time::Duration::from_secs(
                                    opts.retry_delay_secs,
                                ));
                            }
                        }
                    };
                    match res {
                        Ok(s) => {
                            local.a_to_b += s.a_to_b;
                            local.b_to_a += s.b_to_a;
                            local.deleted += s.deleted;
                            local.bytes += s.bytes;
                        }
                        Err(e) => {
                            local.errors += 1;
                            local_errs.push((format!("{:?}", act), e.to_string()));
                        }
                    }
                }
                let mut m = merged.lock().unwrap();
                m.0.a_to_b += local.a_to_b;
                m.0.b_to_a += local.b_to_a;
                m.0.deleted += local.deleted;
                m.0.bytes += local.bytes;
                m.0.errors += local.errors;
                m.1.extend(local_errs);
            });
        }
    });

    let (st, errs) = merged.into_inner().unwrap();
    errors.extend(errs);
    st
}

fn sig_of(be: &dyn Backend, path: &str) -> Option<Sig> {
    be.stat(path).ok().filter(|m| !m.is_dir).map(|m| Sig {
        size: m.size,
        mtime_ms: m.mtime_ms,
        hash: 0,
    })
}

/// Resolve one conflict by copying the chosen side over the other (with a
/// reversible backup of the loser). Returns the new (a, b) signatures so the
/// caller can update the baseline.
pub fn resolve(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    rel: &str,
    keep_a: bool,
    pair: &str,
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    let vdir = versions_dir(pair);
    let pa = join(root_a, rel);
    let pb = join(root_b, rel);
    let throttle = Throttle::new(0);
    let no_cancel = AtomicBool::new(false);
    if keep_a {
        if b.exists(&pb) {
            let _ = back_up(b, &pb, rel, &vdir);
        }
        copy_between(a, &pa, b, &pb, true, &throttle, &no_cancel)?;
    } else {
        if a.exists(&pa) {
            let _ = back_up(a, &pa, rel, &vdir);
        }
        copy_between(b, &pb, a, &pa, true, &throttle, &no_cancel)?;
    }
    Ok((sig_of(a, &pa), sig_of(b, &pb)))
}
