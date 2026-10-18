//! Remote changes into the kernel caches of a FUSE drive (DRV-M4): the
//! sink of the shared change pump (`os/shared/change_pump.rs`).
//!
//! A created or deleted name drops the parent's cached attributes and the
//! name's dentry (`inval_entry`), so the next lookup asks the engine again;
//! a changed file drops its attributes and page cache (`inval_inode`). Only
//! inodes the kernel knows need a notice. Notices go out only from the host
//! loop, never from inside a FUSE request, so they cannot deadlock on a
//! directory lock held by the request they would wait for. A notice the
//! kernel refuses keeps the change queued for the next turn.
use std::ffi::OsStr;
use std::io;

use fuser::{INodeNo, Notifier};

use super::super::shared::change_pump::ChangeSink;
use super::fuse_inodes::{split, InodeTable};

pub(crate) struct FuseSink<'a> {
    pub(crate) notifier: &'a Notifier,
    pub(crate) inodes: &'a InodeTable,
}

impl FuseSink<'_> {
    fn entry_changed(&self, callback_path: &str) -> io::Result<()> {
        let Some((parent, name)) = split(callback_path) else {
            return Ok(());
        };
        let Some(parent) = self.inodes.known(parent) else {
            return Ok(());
        };
        self.notifier
            .inval_entry(INodeNo(parent), OsStr::new(name))?;
        self.notifier.inval_inode(INodeNo(parent), 0, 0)
    }
}

impl ChangeSink for FuseSink<'_> {
    fn created(&mut self, callback_path: &str, _is_directory: bool) -> io::Result<()> {
        self.entry_changed(callback_path)
    }

    fn deleted(&mut self, callback_path: &str, _is_directory: bool) -> io::Result<()> {
        self.entry_changed(callback_path)?;
        // The name no longer resolves; descriptors that are open keep the
        // inode until the kernel forgets it.
        self.inodes.unlinked(callback_path);
        Ok(())
    }

    fn modified(&mut self, callback_path: &str) -> io::Result<()> {
        match self.inodes.known(callback_path) {
            Some(ino) => self.notifier.inval_inode(INodeNo(ino), 0, 0),
            None => Ok(()),
        }
    }
}
