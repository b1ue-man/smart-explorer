//! Seekable background reads share the physical optical-device permit.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::source_media::{self, Priority, Source};

pub(super) struct WallpaperReader {
    file: File,
    source: Option<Source>,
    cancel: Arc<AtomicBool>,
}

impl WallpaperReader {
    pub(super) fn open(path: &Path, cancel: Arc<AtomicBool>) -> io::Result<Self> {
        canceled(&cancel)?;
        let guard = source_media::acquire(path, Priority::Background, &cancel)?;
        let file = File::open(path)?;
        let source = source_media::probe_file(&file)?;
        if source.as_ref() != guard.as_ref().map(|guard| guard.source()) {
            return Err(io::Error::other("wallpaper medium changed while opening"));
        }
        if let Some(guard) = &guard {
            guard.validate_file(&file)?;
        }
        canceled(&cancel)?;
        Ok(Self {
            file,
            source,
            cancel,
        })
    }

    fn access<T>(&mut self, operation: impl FnOnce(&mut File) -> io::Result<T>) -> io::Result<T> {
        canceled(&self.cancel)?;
        // Each bounded read/seek yields the device to waiting foreground work.
        let guard = source_media::acquire_file(&self.file, Priority::Background, &self.cancel)?;
        if guard.as_ref().map(|guard| guard.source()) != self.source.as_ref() {
            return Err(io::Error::other("wallpaper medium changed or was ejected"));
        }
        let result = operation(&mut self.file)?;
        if let Some(guard) = &guard {
            guard.validate_file(&self.file)?;
        }
        canceled(&self.cancel)?;
        Ok(result)
    }
}

impl Read for WallpaperReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let size = buffer.len().min(source_media::OPTICAL_BLOCK_BYTES);
        self.access(|file| file.read(&mut buffer[..size]))
    }
}

impl Seek for WallpaperReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.access(|file| file.seek(position))
    }
}

fn canceled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        // ConnectionAborted is deliberately not Interrupted: decoders must not
        // retry cancellation indefinitely as a transient interrupted read.
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "wallpaper read canceled",
        ))
    } else {
        Ok(())
    }
}
