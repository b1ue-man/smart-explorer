//! Publication of a copy stage with evidence (G3-02c, 2026-10-28;
//! `vfs::promote_copy_stage_evidence`). A stage whose writer reported it
//! (stage_evidence.rs: FSTAT of its exclusively created handle after every
//! WRITE, a regular file of the written size) is renamed without the LSTAT
//! of `validate_staged_file` before it: that FSTAT already proved it. The
//! RENAME (SFTP v3: never replaces) goes out together with an LSTAT of the
//! destination; its answer is the published file's evidence when it shows a
//! regular file of the stage's size, otherwise none and the receipt looks
//! itself (and then reports what it finds). A RENAME the server refused
//! while the LSTAT shows something under the name is `AlreadyExists`, so the
//! engine takes the next numbered name. A stage without a report is
//! published exactly as before (`promote_copy_stage`).
use super::backend::SftpBackend;
use super::channel_pool::note_failure;
use super::enqueue::enqueue;
use super::io_err;
use super::stage_evidence::regular_evidence;
use crate::vfs::{Backend, FileEvidence, VfsResult};
use russh_sftp::client::error::Error as SftpError;
use std::io;

impl SftpBackend {
    pub(super) fn promote_evidenced(
        &self,
        staged: &str,
        destination: &str,
    ) -> VfsResult<Option<FileEvidence>> {
        let Some(proven) = self.written.proven(staged) else {
            return self.promote_copy_stage(staged, destination).map(|()| None);
        };
        let Some(lease) = self.pool.lease(self)? else {
            self.promote_copy_stage(staged, destination)?;
            self.written.forget(staged);
            return Ok(None);
        };
        let channel = lease.channel().clone();
        let rename = {
            let session = channel.session.clone();
            let (from, to) = (staged.to_string(), destination.to_string());
            enqueue(&self.rt, async move { session.rename(from, to).await })
        };
        let look = {
            let session = channel.session.clone();
            let path = destination.to_string();
            enqueue(&self.rt, async move { session.lstat(path).await })
        };
        let renamed = self.rt.block_on(rename);
        let looked = self.rt.block_on(look);
        drop(lease);
        let renamed = renamed.map_err(|error| {
            io::Error::other(format!(
                "SFTP-RENAME von „{staged}“ ohne Ergebnis abgebrochen: {error}"
            ))
        })?;
        let looked = looked.ok().and_then(Result::ok);
        match renamed {
            Ok(_) => {
                self.written.forget(staged);
                Ok(looked
                    .and_then(|attrs| regular_evidence(destination, &attrs.attrs))
                    .filter(|evidence| evidence.size == proven.size))
            }
            Err(error) => {
                note_failure(&self.connection, &channel, &error);
                let answered = matches!(error, SftpError::Status(_));
                if answered && looked.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("„{destination}“ existiert bereits"),
                    ));
                }
                Err(io_err(error))
            }
        }
    }
}
