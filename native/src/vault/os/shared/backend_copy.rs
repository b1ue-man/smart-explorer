//! Copies inside one vault: the storage copies (or streams) the encrypted
//! bytes, since file contents are bound neither to a name nor to a folder
//! in format 8. No cleartext leaves the vault, not even into a spool (K08).
use std::io::{self, Write};
use std::sync::atomic::AtomicBool;

use crate::cryptomator::cleartext_file_size;
use crate::vfs::{self, Backend, VfsMeta};

use super::backend::VaultBackend;
use super::backend_nodes::{invalid_data, invalid_input, Found, NodeKind, Parts};
use super::backend_write::{discard_storage_stage, Promote};

impl VaultBackend {
    /// `server_copy_to_stage`: the storage copies the encrypted file; the
    /// answer is the cleartext length of what it copied.
    pub(super) fn copy_on_storage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &AtomicBool,
    ) -> io::Result<Option<u64>> {
        let _ = size;
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let (found, stored) = self.source_file(src)?;
            let to = self.at(&self.stage_rel_required(&Self::parts(stage)?)?);
            let from = self.at(&found.node.contents());
            let copied = self
                .inner
                .server_copy_to_stage(&from, &to, stored.size, cancel)?;
            let combo = self.session.config().combo;
            Ok(copied.map(|bytes| {
                if bytes == stored.size {
                    cleartext_file_size(bytes, combo).unwrap_or(u64::MAX)
                } else {
                    u64::MAX
                }
            }))
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// A regular file entry and the storage metadata of its content.
    fn source_file(&self, src: &str) -> io::Result<(Found, VfsMeta)> {
        let found = self.find_at(&Self::parts(src)?)?;
        match &found.kind {
            NodeKind::File { contents } => {
                let contents = contents.clone();
                Ok((found, contents))
            }
            _ => Err(invalid_input("Nur Dateien im Tresor lassen sich kopieren")),
        }
    }

    /// `copy_file`: the encrypted bytes into a stage on the storage (server
    /// copy or stream), then published like any write.
    pub(super) fn copy_within(&self, src: &str, dst: &str) -> io::Result<u64> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let (found, stored) = self.source_file(src)?;
            let size = cleartext_file_size(stored.size, self.session.config().combo)?;
            let staged = vfs::unique_staging_path(self, dst, "copy")?;
            let to = self.at(&self.stage_rel_required(&Self::parts(&staged)?)?);
            let from = self.at(&found.node.contents());
            let published = self
                .copy_stored(&from, &to, &stored)
                .and_then(|()| self.promote(&staged, dst, Promote::Replace));
            if published.is_err() {
                discard_storage_stage(&*self.inner, &to);
            }
            published.map(|()| size)
        })();
        result.map_err(|error| self.session.relock(error))
    }

    fn copy_stored(&self, from: &str, to: &str, stored: &VfsMeta) -> io::Result<()> {
        let cancel = AtomicBool::new(false);
        match self
            .inner
            .server_copy_to_stage(from, to, stored.size, &cancel)?
        {
            Some(copied) if copied == stored.size => {}
            Some(_) => return Err(invalid_data("Quelle wurde während des Kopierens geändert")),
            None => {
                let mut reader = self.inner.open_read(from)?;
                let mut writer = self.inner.open_write_copy_stage_sized(to, stored.size)?;
                io::copy(&mut reader, &mut writer)?;
                writer.flush()?;
            }
        }
        let after = self.inner.stat(from)?;
        if after.size != stored.size || after.mtime_ms != stored.mtime_ms {
            return Err(invalid_data("Quelle wurde während des Kopierens geändert"));
        }
        Ok(())
    }
}
