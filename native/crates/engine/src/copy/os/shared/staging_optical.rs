//! The optical source's copy into a stage (moved out of `staging.rs`,
//! A7 2026-10-29, unchanged): one bounded sequential loop, each error tagged
//! with the side it came from.
use std::fs::File;
use std::io;
use std::sync::atomic::AtomicBool;

use super::LocalFailure;

/// Optical bytes use one bounded sequential loop on both desktop systems.
/// Preserve the existing source/target error attribution while sharing the
/// measured block parameter with Windows' metadata-preserving path.
pub(in super::super) fn copy_optical_handles(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    struct SourceReader<'a>(&'a File);
    impl io::Read for SourceReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            io::Read::read(&mut self.0, buffer).map_err(|error| {
                if error.kind() == io::ErrorKind::Interrupted {
                    error
                } else {
                    io::Error::other(LocalFailure::Source(error))
                }
            })
        }
    }
    struct TargetWriter<'a>(&'a mut File);
    impl io::Write for TargetWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            io::Write::write(self.0, bytes).map_err(|error| {
                if error.kind() == io::ErrorKind::Interrupted {
                    error
                } else {
                    io::Error::other(LocalFailure::Target(error))
                }
            })
        }
        fn flush(&mut self) -> io::Result<()> {
            io::Write::flush(self.0).map_err(|error| io::Error::other(LocalFailure::Target(error)))
        }
    }
    crate::source_media::sequential_copy(
        &mut SourceReader(reader),
        &mut TargetWriter(writer),
        cancel,
        progress,
    )
}
