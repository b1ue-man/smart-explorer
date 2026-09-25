//! Upload from a byte stream without a local path (for example a document a
//! content provider hands over): the same reserved destination name, private
//! stage and create-only publication as every other clipboard upload.
use super::copy_commit::{CommitMode, StagedUpload};
use super::entries::validate_transfer_name;
use super::progress::send_transfer_progress;
use super::types::{TransferKind, TransferMsg, TransferProgress};
use super::upload_plan::DestinationNames;
use crate::vfs::remote_util::rjoin;
use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// Uploads `reader` as a new file `name` into `dest_dir` and returns the name
/// it was published under (`name (2).ext` when `name` is taken). The bytes go
/// to a private stage first and are published create-only, so no existing
/// entry is ever replaced. `size_hint` only feeds the progress total: a stream
/// carries no identity that could be re-verified after the copy. Sends
/// progress and exactly one terminal `TransferMsg::Done`, like the other
/// transfer workers.
pub fn upload_reader_progress(
    backend: &dyn crate::vfs::Backend,
    reader: &mut dyn Read,
    size_hint: Option<u64>,
    dest_dir: &str,
    name: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut progress =
        TransferProgress::new(TransferKind::Upload, "Lade hoch", 1, size_hint.unwrap_or(0));
    progress.current = name.to_string();
    let start = Instant::now();
    let mut last = start;
    send_transfer_progress(tx, &progress, &mut last, true);
    let target = StreamTarget {
        backend,
        dest_dir,
        name,
        cancel,
    };
    let result = target.upload(reader, tx, &mut progress, &mut last);
    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    let errors = match &result {
        Ok(_) => {
            progress.files_done = 1;
            Vec::new()
        }
        Err(error) if error == super::cancel::CANCELED_ERROR => Vec::new(),
        Err(error) => {
            progress.errors = 1;
            vec![error.clone()]
        }
    };
    super::cancel::send_done(tx, progress, errors, cancel);
    result
}

struct StreamTarget<'a> {
    backend: &'a dyn crate::vfs::Backend,
    dest_dir: &'a str,
    name: &'a str,
    cancel: &'a AtomicBool,
}

impl StreamTarget<'_> {
    fn upload(
        &self,
        reader: &mut dyn Read,
        tx: &crossbeam_channel::Sender<TransferMsg>,
        progress: &mut TransferProgress,
        last: &mut Instant,
    ) -> Result<String, String> {
        super::cancel::check(self.cancel)?;
        validate_transfer_name(self.name, &rjoin(self.dest_dir, self.name))?;
        let target = DestinationNames::new(self.backend, self.dest_dir).reserve(
            self.backend,
            self.dest_dir,
            self.name,
            self.cancel,
        )?;
        let destination = rjoin(self.dest_dir, &target);
        progress.current = target.clone();
        let mut staged = StagedUpload::open(self.backend, &destination, Some(self.cancel))?;
        let copied = copy_stream(reader, staged.writer(), self.cancel, |bytes| {
            progress.bytes_done = progress.bytes_done.saturating_add(bytes);
            send_transfer_progress(tx, progress, last, false);
        });
        if let Err(error) = copied {
            return Err(staged.failed(error));
        }
        staged.commit(CommitMode::Create, Some(self.cancel), || Ok(()))?;
        Ok(target)
    }
}

fn copy_stream(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        super::cancel::check(cancel)?;
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Quelle lesen: {error}")),
        };
        super::cancel::check(cancel)?;
        if read == 0 {
            return Ok(());
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("Upload schreiben: {error}"))?;
        progress(read as u64);
    }
}
