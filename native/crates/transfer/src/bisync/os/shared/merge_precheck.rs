//! Exact original-content checks and constant-memory byte comparisons.
use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::merge_recorded::OriginalContent;
use super::merge_recovery::Recovery;
use super::types::{PairSide, Sig};
use crate::vfs::Backend;
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}
pub(super) fn checked_original(
    backend: &dyn Backend,
    path: &str,
    original: OriginalContent<'_>,
    recovery: &Recovery,
    side: PairSide,
    merged: &[u8],
    recovering: bool,
    cancel: &AtomicBool,
) -> io::Result<(CapturedFile, Option<Sig>, bool)> {
    if original.signature.is_some() != original.bytes.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "original text/signature presence differs",
        ));
    }
    let current = capture(backend, path, ExpectedFile::Unknown, "merge original")?;
    let done = if side == PairSide::A {
        recovery.done_a
    } else {
        recovery.done_b
    };
    let Some(meta) = current.metadata.as_ref() else {
        if original.signature.is_none() && !done {
            return Ok((current, None, false));
        }
        return Err(drift("merge original disappeared"));
    };
    current.regular("merge original")?;
    let mut reader = crate::vfs::open_read_regular(backend, path, meta.id.as_deref())?;
    let mut compared = Compared::new(original.bytes.unwrap_or_default(), merged);
    let actual =
        super::transfer_stream::stream(&mut *reader, &mut compared, cancel, None, 0, |_| {})?;
    drop(reader);
    revalidate(backend, path, &current, "merge original")?;
    let signature = Sig {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        hash: actual.hash(),
    };
    if (recovering || done)
        && (done || original.bytes.is_none_or(|bytes| bytes != merged))
        && compared.second_matches()
        && actual.hex() == recovery.merged
    {
        super::apply_stage::require_durable(super::apply_stage::namespace(backend, path)?)?;
        return Ok((current, Some(signature), true));
    }
    let expected = original
        .signature
        .ok_or_else(|| drift("merge original was recreated"))?;
    let bytes = original
        .bytes
        .ok_or_else(|| drift("merge original was recreated"))?;
    if done
        || (!recovering && (meta.size != expected.size || meta.mtime_ms != expected.mtime_ms))
        || actual.bytes != bytes.len() as u64
        || !compared.first_matches()
    {
        return Err(drift("original text changed since the merge was loaded"));
    }
    Ok((current, Some(signature), false))
}

pub(super) struct Compared<'a> {
    first: &'a [u8],
    second: &'a [u8],
    offset: usize,
    matches: [bool; 2],
}
impl<'a> Compared<'a> {
    pub(super) fn new(first: &'a [u8], second: &'a [u8]) -> Self {
        Self {
            first,
            second,
            offset: 0,
            matches: [true; 2],
        }
    }
    pub(super) fn first_matches(&self) -> bool {
        self.matches[0] && self.offset == self.first.len()
    }
    pub(super) fn second_matches(&self) -> bool {
        self.matches[1] && self.offset == self.second.len()
    }
}
impl std::io::Write for Compared<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .offset
            .checked_add(bytes.len())
            .ok_or_else(|| drift("merge size overflow"))?;
        self.matches[0] &= self.first.get(self.offset..end) == Some(bytes);
        self.matches[1] &= self.second.get(self.offset..end) == Some(bytes);
        self.offset = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
