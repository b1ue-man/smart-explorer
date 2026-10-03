//! FEAT-negotiated MLSx, single-entry probes and a dot-complete LIST
//! fallback. A command refusal is not a missing object.
use super::core_impl::{basename, dir_meta, parent_dir, parse_list_line_unchecked};
use super::errors::{command_path, map, reply_code};
use super::io_adapters::FtpConnection;
use crate::vfs::{MtimePrecision, OmissionReason, VfsListing, VfsMeta, VfsOmission};
use std::io;
use suppaftp::{RustlsFtpStream, Status};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Features { pub(super) mlst: bool, pub(super) mlsd: bool, pub(super) mfmt: bool }

impl FtpConnection {
    pub(super) fn features(&self) -> io::Result<Features> {
        if let Some(features) = *self.features.lock().map_err(|_| io::Error::other("FTP FEAT lock poisoned"))? {
            return Ok(features);
        }
        let features = self.with_stream_read(|stream| match stream.feat() {
            Ok(features) => {
                let has = |name: &str| features.keys().any(|key| key.eq_ignore_ascii_case(name));
                // RFC 3659 advertises MLST for both MLST and MLSD.
                Ok(Features { mlst: has("MLST"), mlsd: has("MLST") || has("MLSD"), mfmt: has("MFMT") })
            }
            Err(error) if matches!(reply_code(&error), Some(500 | 502 | 504)) => Ok(Features::default()),
            Err(error) => Err(map(error)),
        })?;
        *self.features.lock().map_err(|_| io::Error::other("FTP FEAT lock poisoned"))? = Some(features);
        Ok(features)
    }

    pub(super) fn observed_precision(&self) -> MtimePrecision {
        self.precision.lock().map(|precision| *precision).unwrap_or(MtimePrecision::Unknown)
    }
}

pub(super) fn parse_time(text: &str) -> Option<i64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.len() != 14 || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit()) { return None }
    let date = chrono::NaiveDateTime::parse_from_str(whole, "%Y%m%d%H%M%S").ok()?;
    let mut millis = 0i64;
    for index in 0..3 { millis = millis * 10 + fraction.as_bytes().get(index).map_or(0, |b| i64::from(*b - b'0')); }
    date.and_utc().timestamp_millis().checked_add(millis)
}

/// Split at the FIRST fact/name space: semicolons and extra spaces belong
/// to the literal name. MLST's name may be absolute; its queried name wins.
pub(super) fn parse_mlsx(line: &str, queried: Option<&str>) -> io::Result<Option<VfsMeta>> {
    let (facts, raw_name) = line.split_once(' ').ok_or_else(|| invalid("FTP MLSx row lacks a name"))?;
    let name = queried.unwrap_or(raw_name).to_string();
    let mut meta = VfsMeta { hidden: name.starts_with('.'), name, ..VfsMeta::default() };
    let mut kind = None;
    let mut has_size = false;
    for fact in facts.split(';').filter(|fact| !fact.is_empty()) {
        let (key, value) = fact.split_once('=').ok_or_else(|| invalid("FTP MLSx fact lacks '='"))?;
        match key.to_ascii_lowercase().as_str() {
            "type" => kind = Some(value.to_ascii_lowercase()),
            "size" => {
                meta.size = value.parse().map_err(|_| invalid("FTP MLSx size is not u64"))?;
                has_size = true;
            }
            "sizd" => {}
            "modify" => meta.mtime_ms = parse_time(value).ok_or_else(|| invalid("FTP MLSx time is invalid"))?,
            _ => {}
        }
    }
    match kind.as_deref() {
        Some("cdir" | "pdir") => return Ok(None),
        Some("dir") => { meta.is_dir = true; meta.size = 0; }
        Some("file") if has_size => {}
        Some("file") => return Err(invalid("FTP MLSx regular file lacks its size fact")),
        Some(kind) if kind == "link" || kind.starts_with("os.unix=symlink") || kind.starts_with("os.unix=slink") => meta.is_symlink = true,
        Some(_) => { meta.special = true; meta.size = 0; }
        None => return Err(invalid("FTP MLSx lacks a type fact")),
    }
    Ok(Some(meta))
}

pub(super) fn list(connection: &FtpConnection, folder: &str) -> io::Result<VfsListing> {
    list_mode(connection, folder, false)
}

/// Old servers may refuse LIST -a. Browsing retains plain LIST support;
/// sync and absence proofs require the stronger enumeration above.
pub(super) fn list_browse(connection: &FtpConnection, folder: &str) -> io::Result<VfsListing> {
    list_mode(connection, folder, true)
}

fn list_mode(connection: &FtpConnection, folder: &str, allow_plain: bool) -> io::Result<VfsListing> {
    command_path(folder)?;
    let features = connection.features()?;
    let lines = connection.with_stream_read(|stream| {
        if features.mlsd {
            match stream.mlsd(Some(folder)) {
                Ok(lines) => return Ok((true, lines)),
                Err(error) if matches!(reply_code(&error), Some(500 | 502 | 504)) => {}
                Err(error) => return Err(map(error)),
            }
        }
        // CWD prevents LIST option/glob interpretation of the directory
        // itself. Restore before releasing this control connection.
        let original = stream.pwd().map_err(map)?;
        stream.cwd(folder).map_err(map)?;
        let listed = match stream.list(Some("-a")) {
            Err(error) if allow_plain && matches!(reply_code(&error), Some(500 | 501 | 502 | 504 | 550)) => {
                stream.list(None).map_err(map)
            }
            other => other.map_err(map),
        };
        let restored = stream.cwd(&original).map_err(map);
        restored?;
        listed.map(|lines| (false, lines))
    })?;
    let (mlsx, lines) = lines;
    let mut out = VfsListing::default();
    let mut names = std::collections::HashSet::new();
    for line in lines {
        if !mlsx && line.starts_with("total ") { continue }
        let name_hint = if mlsx { line.split_once(' ').map(|(_, name)| name.to_string()) }
            else { list_name(&line) };
        let parsed = if mlsx { parse_mlsx(&line, None) } else { parse_list_line_unchecked(&line).map(Some) };
        let entry = match parsed {
            Ok(None) => continue,
            Ok(Some(meta)) => meta,
            Err(error) => {
                let rel = name_hint.ok_or_else(|| invalid(format!("FTP row has no identifiable child: {error}")))?;
                if !names.insert(rel.clone()) { return Err(invalid("FTP child/omission names collide")) }
                out.omitted.push(VfsOmission { rel, reason: OmissionReason::Unreadable, detail: error.to_string() });
                continue;
            }
        };
        if matches!(entry.name.as_str(), "." | "..") { continue }
        if !names.insert(entry.name.clone()) { return Err(invalid("FTP child names collide")) }
        if entry.name.contains('\u{fffd}') || crate::vfs::validate_child_name(&entry.name).is_err()
            || command_path(&entry.name).is_err() {
            out.omitted.push(VfsOmission { rel: entry.name, reason: OmissionReason::Unrepresentable,
                detail: "FTP child cannot be addressed by this path/control interface".into() });
        } else { out.entries.push(entry); }
    }
    if let Ok(mut precision) = connection.precision.lock() {
        *precision = if mlsx && out.entries.iter().all(|entry| entry.mtime_ms != 0) { MtimePrecision::Seconds }
            else if mlsx { MtimePrecision::Unknown } else { MtimePrecision::Days };
    }
    if !mlsx {
        if let Ok(mut cached) = connection.features.lock() {
            if let Some(features) = cached.as_mut() { features.mlsd = false; }
        }
    }
    Ok(out)
}

fn list_name(line: &str) -> Option<String> {
    let mut start = 0usize;
    for _ in 0..8 {
        start += line.get(start..)?.find(|c: char| !c.is_ascii_whitespace())?;
        start += line.get(start..)?.find(|c: char| c.is_ascii_whitespace())?;
    }
    start += line.get(start..)?.find(|c: char| !c.is_ascii_whitespace())?;
    Some(line.get(start..)?.split_once(" -> ").map_or(&line[start..], |(name, _)| name).to_string())
}

pub(super) fn stat(connection: &FtpConnection, path: &str) -> io::Result<VfsMeta> {
    command_path(path)?;
    if path == "/" || path.is_empty() { return Ok(dir_meta("/".into())) }
    let base = basename(path);
    let features = connection.features()?;
    let direct = connection.with_stream_read(|stream| {
        if features.mlst {
            match stream.mlst(Some(path)) {
                Ok(line) => {
                    // A server may omit default MLST facts. A separate SIZE
                    // and MDTM can still prove this one regular file.
                    if let Ok(metadata) = parse_mlsx(&line, Some(&base)) { return Ok(metadata) }
                }
                // Even 550 can mean denied; a complete parent listing may
                // resolve absence without treating that reply as proof.
                Err(error) if matches!(reply_code(&error), Some(500 | 502 | 504 | 550)) => {}
                Err(error) => return Err(map(error)),
            }
        }
        match stream.custom_command(format!("SIZE {path}"), &[Status::File]) {
            Ok(response) => {
                let reply = response.as_string().map_err(|error| invalid(error.to_string()))?;
                let size = reply.split_ascii_whitespace().nth(1).and_then(|s| s.parse::<u64>().ok())
                    .ok_or_else(|| invalid("FTP SIZE response is not u64"))?;
                let time = read_time(stream, path)?;
                Ok(Some(VfsMeta { name: base.clone(), hidden: base.starts_with('.'), size,
                    mtime_ms: time.unwrap_or(0), ..VfsMeta::default() }))
            }
            Err(error) if matches!(reply_code(&error), Some(500 | 502 | 504 | 550)) => Ok(None),
            Err(error) => Err(map(error)),
        }
    })?;
    if let Some(meta) = direct { return Ok(meta) }
    let listed = list(connection, &parent_dir(path))?;
    if let Some(omission) = listed.omitted.iter().find(|entry| entry.rel == base) {
        return Err(invalid(omission.detail.clone()));
    }
    listed.entries.into_iter().find(|meta| meta.name == base)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path} nicht gefunden")))
}

pub(super) fn read_time(stream: &mut RustlsFtpStream, path: &str) -> io::Result<Option<i64>> {
    match stream.custom_command(format!("MDTM {path}"), &[Status::File]) {
        Ok(response) => {
            let reply = response.as_string().map_err(|error| invalid(error.to_string()))?;
            let token = reply.split_ascii_whitespace().nth(1).unwrap_or("");
            parse_time(token).map(Some).ok_or_else(|| invalid("FTP MDTM response has no UTC time"))
        }
        Err(error) if matches!(reply_code(&error), Some(500 | 502 | 504 | 550)) => Ok(None),
        Err(error) => Err(map(error)),
    }
}

fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.into()) }
