use super::{acquire, acquire_source, permit::context_canceled, probe, Guard, Priority, ReadMode};
use std::{io::{self, Read}, path::{Path, PathBuf}, sync::atomic::AtomicBool};

/// Acquires before opening, preserving the caller's existing safe handle-open
/// contract. The guard covers the stream until EOF/drop, including offsets.
pub fn guard_reader<R: Read + Send + 'static>(path: &Path, open: impl FnOnce() -> io::Result<R>) -> io::Result<Box<dyn Read + Send>> {
    let guard = match probe(path)? {
        Some(source) => Some(acquire_source(path, source, Priority::Foreground, context_canceled)?),
        None => None,
    };
    let reader = open()?;
    let source = guard.as_ref().map(|guard| guard.source().clone());
    Ok(Box::new(Reader { reader, guard, source, path: path.to_path_buf(), held: None }))
}

/// Same reader guarantee, with validation bound to a retained file object.
/// `file` must be a clone of the underlying reader handle (shared cursor).
/// This also detects the physical end below a buffering/decryption wrapper.
pub fn guard_file_reader<R: Read + Send + 'static>(file: std::fs::File, reader: R) -> io::Result<Box<dyn Read + Send>> {
    let guard = super::acquire_file_current(&file, Priority::Foreground)?;
    let source = guard.as_ref().map(|guard| guard.source().clone());
    Ok(Box::new(Reader { reader, guard, source, path: PathBuf::new(), held: Some(file) }))
}

struct Reader<R> { reader: R, guard: Option<Guard>, source: Option<super::Source>, path: PathBuf, held: Option<std::fs::File> }
impl<R: Read> Read for Reader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if context_canceled() { return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "optical read canceled")); }
        if buffer.is_empty() { return Ok(0); }
        if self.guard.is_none() {
            if let Some(source) = &self.source {
                self.guard = Some(match &self.held {
                    Some(file) => super::permit::acquire_checked(source.clone(), Priority::Foreground, context_canceled, || super::probe_file(file))?,
                    None => acquire_source(&self.path, source.clone(), Priority::Foreground, context_canceled)?,
                });
            }
        }
        if let Some(guard) = &self.guard {
            match &self.held { Some(file) => guard.validate_file(file)?, None => guard.validate(&self.path)? };
            guard.set_mode(ReadMode::Sequential);
            let size = buffer.len().min(super::OPTICAL_BLOCK_BYTES);
            let count = self.reader.read(&mut buffer[..size])?;
            // EOF ends the physical read. A subsequent metadata validation
            // can acquire independently, without reentering our own lock.
            let physical_end = match &mut self.held {
                Some(file) => std::io::Seek::stream_position(file)? >= file.metadata()?.len(),
                None => false,
            };
            if count == 0 || physical_end { self.guard.take(); }
            return Ok(count);
        }
        self.reader.read(buffer)
    }
}

/// Directory iterators may perform I/O on next(), not just read_dir(). Each
/// next/open operation is bounded and background priority is surrendered
/// before returning an item. The caller's cancellation flag remains authoritative.
pub struct GuardedIterator<'a, I> {
    inner: I,
    path: PathBuf,
    priority: Priority,
    cancel: &'a AtomicBool,
    failed: bool,
    source: Option<super::Source>,
}

impl<'a, I> GuardedIterator<'a, I> {
    pub fn new(path: &Path, priority: Priority, cancel: &'a AtomicBool, open: impl FnOnce() -> io::Result<I>) -> io::Result<Self> {
        let _guard = acquire(path, priority, &cancel)?;
        let source = _guard.as_ref().map(|guard| guard.source().clone());
        let inner = open()?;
        Ok(Self { inner, path: path.to_path_buf(), priority, cancel, failed: false, source })
    }
}

impl<T, I: Iterator<Item = io::Result<T>>> Iterator for GuardedIterator<'_, I> {
    type Item = io::Result<T>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed { return None; }
        let _guard = match acquire(&self.path, self.priority, &self.cancel) {
            Ok(guard) => guard,
            Err(error) => { self.failed = true; return Some(Err(error)); }
        };
        if _guard.as_ref().map(|guard| guard.source()) != self.source.as_ref() {
            self.failed = true;
            return Some(Err(io::Error::other("optical medium changed during enumeration")));
        }
        self.inner.next()
    }
}
