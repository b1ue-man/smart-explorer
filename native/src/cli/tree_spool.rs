use super::tree_plan::TransferPlan;
use crate::vfs::Backend;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub(super) struct SourceSpool {
    files: Vec<Option<SpoolFile>>,
    _directory: tempfile::TempDir,
}

struct SpoolFile {
    path: PathBuf,
    expected_size: u64,
}

impl SourceSpool {
    pub(super) fn collect(source: &dyn Backend, plan: &TransferPlan) -> Result<Self, String> {
        // Re-list the exact tree before opening any source file. All output
        // below is private local staging, never a destination-backend path.
        plan.validate_source_tree(source)?;
        let directory = tempfile::Builder::new()
            .prefix("smart-explorer-cli-tree-")
            .tempdir()
            .map_err(|error| format!("cannot create private transfer spool: {error}"))?;
        let mut files: Vec<Option<SpoolFile>> = std::iter::repeat_with(|| None)
            .take(plan.entries.len())
            .collect();

        for (index, entry) in plan.entries.iter().enumerate() {
            if entry.source.is_dir {
                continue;
            }
            plan.validate_source_entry(source, index)?;
            let path = directory.path().join(format!("{index:016x}.part"));
            let mut writer = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| format!("cannot create private transfer spool file: {error}"))?;
            let reader = source
                .open_read_id(&entry.source_path, entry.source.id.as_deref())
                .map_err(|error| format!("cannot read source {}: {error}", entry.source_path))?;
            copy_exact(reader, &mut writer, entry.source.size, &entry.source_path)?;
            writer
                .flush()
                .map_err(|error| format!("cannot flush private transfer spool: {error}"))?;
            writer
                .sync_all()
                .map_err(|error| format!("cannot sync private transfer spool: {error}"))?;
            drop(writer);
            plan.validate_source_entry(source, index)?;
            files[index] = Some(SpoolFile {
                path,
                expected_size: entry.source.size,
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
    ) -> Result<u64, String> {
        let file = self
            .files
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| "transfer plan has no spooled content for a file".to_string())?;
        let before = std::fs::symlink_metadata(&file.path)
            .map_err(|error| format!("cannot inspect private transfer spool: {error}"))?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || before.len() != file.expected_size
        {
            return Err("private transfer spool changed before publication".to_string());
        }
        let reader = File::open(&file.path)
            .map_err(|error| format!("cannot reopen private transfer spool: {error}"))?;
        let copied = copy_exact(reader, writer, file.expected_size, "private transfer spool")?;
        let after = std::fs::symlink_metadata(&file.path)
            .map_err(|error| format!("cannot recheck private transfer spool: {error}"))?;
        if !after.is_file() || after.file_type().is_symlink() || after.len() != file.expected_size {
            return Err("private transfer spool changed during publication".to_string());
        }
        Ok(copied)
    }
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
