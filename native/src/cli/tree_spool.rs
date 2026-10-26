use super::tree_observer::{Counter, TreeObserver};
use super::tree_plan::TransferPlan;
use crate::cryptomator::{
    CipherCombo, ContentCryptor, DecryptingReader, EncryptingWriter, Masterkey, MasterkeyCell,
};
use crate::vfs::Backend;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub(super) struct SourceSpool {
    files: Vec<Option<SpoolFile>>,
    _directory: tempfile::TempDir,
}

/// Content never reaches temporary storage in plaintext. This also covers
/// a vault mounted through a local locator, whose backend label is local.
struct SpoolFile {
    path: PathBuf,
    cryptor: ContentCryptor,
    expected_size: u64,
    stored_size: u64,
}

impl SpoolFile {
    fn with_plain<T>(
        &self,
        read: impl FnOnce(&mut dyn Read) -> Result<T, String>,
    ) -> Result<T, String> {
        let file =
            crate::local_access::open_regular(&self.path, crate::local_access::FinalLink::Refuse)
                .map_err(|error| error.to_string())?;
        if file.metadata().map_err(|error| error.to_string())?.len() != self.stored_size {
            return Err("Privater Quellspool wurde geändert".into());
        }
        let mut reader =
            DecryptingReader::new(file, self.cryptor.clone()).map_err(|error| error.to_string())?;
        read(&mut reader)
    }
}

impl SourceSpool {
    pub(super) fn matches(
        &self,
        index: usize,
        backend: &dyn Backend,
        path: &str,
        id: Option<&str>,
        observer: &TreeObserver,
    ) -> Result<bool, String> {
        crate::source_media::with_cancellation(observer.cancel_flag(), || {
            let file = self
                .files
                .get(index)
                .and_then(Option::as_ref)
                .ok_or_else(|| "Quellspool fehlt für die Inhaltsbestätigung".to_string())?;
            let expected_hash =
                file.with_plain(|reader| content_hash(reader, file.expected_size, observer))?;
            let reader = crate::vfs::open_read_regular(backend, path, id)
                .map_err(|error| error.to_string())?;
            let actual_hash = content_hash(reader, file.expected_size, observer)?;
            Ok(expected_hash.is_some() && expected_hash == actual_hash)
        })
    }

    pub(super) fn collect(
        source: &dyn Backend,
        plan: &TransferPlan,
        observer: &TreeObserver,
    ) -> Result<Self, String> {
        crate::source_media::with_cancellation(observer.cancel_flag(), || {
            Self::collect_inner(source, plan, observer)
        })
    }

    fn collect_inner(
        source: &dyn Backend,
        plan: &TransferPlan,
        observer: &TreeObserver,
    ) -> Result<Self, String> {
        // Re-list the exact tree before opening any source file. All output
        // below is private local staging, never a destination-backend path.
        plan.validate_source_tree(source)?;
        let directory = tempfile::Builder::new()
            .prefix("smart-explorer-cli-tree-")
            .tempdir()
            .map_err(|error| error.to_string())?;
        let mut files: Vec<Option<SpoolFile>> = std::iter::repeat_with(|| None)
            .take(plan.entries.len())
            .collect();

        for (index, entry) in plan.entries.iter().enumerate() {
            if entry.source.is_dir {
                continue;
            }
            observer.check()?;
            plan.validate_source_entry(source, index)?;
            let path = directory.path().join(format!("{index:016x}.sealed"));
            let mut spool = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| format!("cannot create encrypted transfer spool: {error}"))?;
            let key = Masterkey::generate().map_err(|error| error.to_string())?;
            let cryptor = ContentCryptor::new(MasterkeyCell::new(key), CipherCombo::SivGcm);
            let limit = entry
                .source
                .size
                .checked_add(1)
                .ok_or_else(|| "source size is too large to verify".to_string())?;
            let copied = {
                let reader = source
                    .open_read_id(&entry.source_path, entry.source.id.as_deref())
                    .map_err(|error| {
                        format!("cannot read source {}: {error}", entry.source_path)
                    })?;
                let mut bounded = observer.reader(reader, Counter::Read).take(limit);
                let mut writer = EncryptingWriter::new(&mut spool, cryptor.clone())
                    .map_err(|error| error.to_string())?;
                let copied = io::copy(&mut bounded, &mut writer)
                    .map_err(|error| observer.explain(error.to_string()))?;
                writer.finish().map_err(|error| error.to_string())?;
                copied
            };
            // The source reader and its optical-device lease have ended before
            // revalidating source metadata below.
            if copied != entry.source.size {
                return Err(format!(
                    "source length changed while spooling {}",
                    entry.source_path
                ));
            }
            spool.sync_all().map_err(|error| error.to_string())?;
            let stored_size = spool.metadata().map_err(|error| error.to_string())?.len();
            drop(spool);
            plan.validate_source_entry(source, index)?;
            observer.file_done(Counter::Read);
            files[index] = Some(SpoolFile {
                path,
                cryptor,
                expected_size: entry.source.size,
                stored_size,
            });
        }
        // A late tree/listing fault or a node added while earlier files were
        // read still fails before the destination namespace is touched.
        plan.validate_source_tree(source)?;
        Ok(Self {
            files,
            _directory: directory,
        })
    }

    pub(super) fn copy_file_into(
        &self,
        index: usize,
        writer: &mut dyn Write,
        observer: &TreeObserver,
    ) -> Result<u64, String> {
        let file = self
            .files
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| "transfer plan has no spooled content for a file".to_string())?;
        file.with_plain(|reader| {
            copy_exact(
                observer.reader(reader, Counter::Written),
                writer,
                file.expected_size,
                "private encrypted transfer spool",
            )
            .map_err(|error| observer.explain(error))
        })
    }
}

fn content_hash(
    mut reader: impl Read,
    expected: u64,
    observer: &TreeObserver,
) -> Result<Option<Vec<u8>>, String> {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut remaining = expected;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        observer.check()?;
        let wanted = remaining.min(buffer.len() as u64) as usize;
        let read = reader
            .read(&mut buffer[..wanted])
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(None);
        }
        context.update(&buffer[..read]);
        remaining -= read as u64;
    }
    observer.check()?;
    let mut extra = [0u8; 1];
    if reader.read(&mut extra).map_err(|error| error.to_string())? != 0 {
        return Ok(None);
    }
    Ok(Some(context.finish().as_ref().to_vec()))
}

fn copy_exact(
    reader: impl Read,
    writer: &mut dyn Write,
    expected: u64,
    label: &str,
) -> Result<u64, String> {
    let limit = expected
        .checked_add(1)
        .ok_or_else(|| format!("source size is too large to verify: {label}"))?;
    let copied = io::copy(&mut reader.take(limit), writer)
        .map_err(|error| format!("source read failed for {label}: {error}"))?;
    if copied < expected {
        return Err(format!(
            "source ended early while reading {label}: expected {expected} bytes, read {copied}"
        ));
    }
    if copied > expected {
        return Err(format!(
            "source grew while reading {label}: expected {expected} bytes, read at least {copied}"
        ));
    }
    Ok(copied)
}
