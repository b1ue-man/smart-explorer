//! Exact regular-file facts shared by LIST refinement and standalone stat.
//! Never enumerate here: callers own any complete-parent absence proof.
use super::errors::{map, reply_code};
use super::metadata::read_time;
use crate::vfs::VfsMeta;
use std::io;
use suppaftp::{RustlsFtpStream, Status};

pub(super) fn regular_file(
    stream: &mut RustlsFtpStream,
    path: &str,
    name: &str,
) -> io::Result<Option<VfsMeta>> {
    let response = match stream.custom_command(format!("SIZE {path}"), &[Status::File]) {
        Ok(response) => response,
        Err(error) if matches!(reply_code(&error), Some(500 | 502 | 504 | 550)) => return Ok(None),
        Err(error) => return Err(map(error)),
    };
    let reply = response
        .as_string()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let size = reply
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "FTP SIZE response is not u64")
        })?;
    // If MDTM is unavailable, both listing and stat must retain the same
    // parent-LIST facts, rather than combining LIST time with a zero-time stat.
    let Some(mtime_ms) = read_time(stream, path)? else {
        return Ok(None);
    };
    Ok(Some(VfsMeta {
        name: name.to_string(),
        hidden: name.starts_with('.'),
        size,
        mtime_ms,
        ..VfsMeta::default()
    }))
}
