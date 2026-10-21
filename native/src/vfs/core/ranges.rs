//! Parts of one large file: the stage several writers fill at once, and the
//! default of the range read every `Backend` has (`Backend::open_read_range`).
use std::io::{Read, Write};

use super::{Backend, VfsResult};

/// A stage that several writers fill at the same time, each its own range.
/// Only backends that can write at an offset into one stage provide it
/// (`Backend::open_segmented_stage`); publishing stays the engine's job.
pub trait SegmentedStage: Send + Sync {
    /// Path of the stage the engine publishes. Usually the name that was
    /// passed; a host that picks the name itself (Share, Agent) returns its own.
    fn stage_path(&self) -> &str;

    /// Writer for exactly `[offset, offset + len)`: its `flush` fails unless
    /// exactly `len` bytes came. Any number of them may be open at once.
    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>>;

    /// After the `flush` of every writer: checks that the stage is `size`
    /// bytes long and closes its handles. Does not publish.
    fn finish(self: Box<Self>) -> VfsResult<()>;

    /// Gives up: removes the stage (only when this client created it) and
    /// closes everything. Failures are not reported.
    fn abort(self: Box<Self>);
}

/// `Backend::open_read_range` by default: the backend reads from an offset
/// (`open_read_at`) and the reader is cut after `len` bytes; a backend that
/// says it cannot read ranges answers `Ok(None)`.
pub(super) fn default_open_read_range<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    id: Option<&str>,
    offset: u64,
    len: u64,
) -> VfsResult<Option<Box<dyn Read + Send>>> {
    if !backend.supports_range_reads(path) {
        return Ok(None);
    }
    Ok(backend
        .open_read_at(path, id, offset)?
        .map(|reader| Box::new(reader.take(len)) as Box<dyn Read + Send>))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{Scheme, VfsMeta};
    use std::io::{self, Cursor};

    const DATA: &[u8] = b"0123456789abcdef";

    /// Reads from an offset of a small in-memory file, if told it can.
    struct Memory(bool);

    impl Backend for Memory {
        fn scheme(&self) -> Scheme {
            Scheme::Local
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
            Ok(Vec::new())
        }
        fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
            Err(io::ErrorKind::NotFound.into())
        }
        fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
            Ok(Box::new(Cursor::new(DATA)))
        }
        fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
            Ok(Box::new(io::sink()))
        }
        fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_file(&self, _path: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_dir(&self, _path: &str) -> VfsResult<()> {
            Ok(())
        }
        fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
            Ok(())
        }
        fn supports_range_reads(&self, _path: &str) -> bool {
            self.0
        }
        fn open_read_at(
            &self,
            _path: &str,
            _id: Option<&str>,
            offset: u64,
        ) -> VfsResult<Option<Box<dyn Read + Send>>> {
            let mut cursor = Cursor::new(DATA);
            cursor.set_position(offset);
            Ok(Some(Box::new(cursor)))
        }
    }

    #[test]
    fn speed_task_vfs_default_open_read_range_takes_len() {
        let mut window = Vec::new();
        let mut reader = Memory(true)
            .open_read_range("f", None, 4, 5)
            .unwrap()
            .unwrap();
        reader.read_to_end(&mut window).unwrap();
        assert_eq!(window, b"45678", "exactly len bytes from the offset");
        let mut tail = Vec::new();
        let mut reader = Memory(true)
            .open_read_range("f", None, 12, 99)
            .unwrap()
            .unwrap();
        reader.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, b"cdef", "fewer only at the end of the file");
        assert!(Memory(false)
            .open_read_range("f", None, 0, 4)
            .unwrap()
            .is_none());
        assert!(Memory(true)
            .open_segmented_stage("s", 16)
            .unwrap()
            .is_none());
        let identity = Memory(true).flow_identity("f");
        assert_eq!(identity, format!("{:?}|{}", Scheme::Local, "Local:/"));
    }
}
