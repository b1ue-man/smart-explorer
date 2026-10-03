//! Filesystem adapters for Linux and Android storage that lacks primitives
//! the local adapters rely on: the no-replace rename ladder serves NFS, FUSE
//! (sshfs, ntfs-3g, FAT through FUSE) and Android's MediaProvider FUSE
//! storage, and is used by the VFS and the copy module on both platforms.
#[path = "os/rename.rs"]
mod rename;
#[cfg(test)]
#[path = "os/rename_tests.rs"]
mod rename_tests;

pub(crate) use rename::rename_no_replace;
