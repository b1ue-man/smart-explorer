//! [`TextSource`] over a local file: positioned reads through the platform
//! adapter, so a document of any size never loads the file whole.
use std::fs::File;
use std::io;
use std::path::Path;

use crate::textview::TextSource;

/// A local file as random-access bytes; its length is taken when opened.
#[derive(Debug)]
pub struct FileSource {
    file: File,
    len: u64,
}

impl FileSource {
    /// Opens `path` for reading; a folder or special file is refused.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} ist keine Datei", path.display()),
            ));
        }
        Ok(Self {
            file,
            len: meta.len(),
        })
    }
}

impl TextSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let room = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let n = buf.len().min(room);
        super::platform::read_at(&self.file, offset, &mut buf[..n])
    }
}
