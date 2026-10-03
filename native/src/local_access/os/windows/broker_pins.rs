//! Read-only directory pins for an existing, authenticated local grant.
use super::super::{directory_handle::DirectoryHandle, pipe::Pipe, privilege::BackupRead};
use super::{duplicate_file, error_code, Client};
use crate::local_access::protocol::{self, PinReply, ReadKind, ReadRequest};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::PathBuf,
    ptr::null_mut,
    sync::atomic::Ordering,
    time::Duration,
};
use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_CLOSE_SOURCE, HANDLE};

pub(crate) struct DirectoryPins {
    pub(crate) path: PathBuf,
    pub(crate) files: Vec<File>,
}

impl Client {
    pub(super) fn pin(&self, path: String, all_ancestors: bool) -> io::Result<DirectoryPins> {
        let channel = self
            .channel
            .lock()
            .map_err(|_| io::Error::other("Lesehelfer-Verbindung unterbrochen"))?;
        if !self.alive() {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let peer = self.child.as_raw_handle();
        channel
            .send(
                &ReadRequest {
                    path,
                    kind: if all_ancestors {
                        ReadKind::PinRoot
                    } else {
                        ReadKind::PinChild
                    },
                },
                peer,
            )
            .inspect_err(|_| self.healthy.store(false, Ordering::Release))?;
        let reply: PinReply = channel
            .receive(peer, Duration::from_secs(30))
            .inspect_err(|_| self.healthy.store(false, Ordering::Release))?;
        // Adopt all duplicates first so a later validation failure closes them.
        let mut files = Vec::new();
        for raw in reply.handles {
            let raw =
                usize::try_from(raw).map_err(|_| io::Error::other("Ungültiger Ordner-Pin"))?;
            if raw == 0 || raw == usize::MAX {
                return Err(io::Error::other("Ungültiger Ordner-Pin"));
            }
            files.push(unsafe { File::from_raw_handle(raw as HANDLE) });
        }
        if let Some(error) = reply.error {
            return Err(io::Error::from_raw_os_error(error));
        }
        if files.is_empty()
            || (!all_ancestors && files.len() != 1)
            || reply.path.is_empty()
            || reply.path.len() > 32767
            || reply.path.contains(&0)
        {
            return Err(io::Error::other("Ungültige gepinnte Verzeichnisantwort"));
        }
        let path = PathBuf::from(OsString::from_wide(&reply.path));
        if !path.is_absolute() {
            return Err(io::Error::other("Lesehelfer lieferte einen relativen Pfad"));
        }
        Ok(DirectoryPins { path, files })
    }
}

pub(super) struct GrantedRoot {
    logical: String,
    directory: DirectoryHandle,
}

impl GrantedRoot {
    pub(super) fn open(logical: &str) -> io::Result<Self> {
        protocol::validate_root(logical)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let _backup = BackupRead::enable()?;
        // Only the authorized root follows links. Keep its entire physical
        // ancestry pinned for the helper session, even between requests.
        let directory = DirectoryHandle::open_root(std::path::Path::new(logical))?;
        Ok(Self {
            logical: logical.to_owned(),
            directory,
        })
    }

    fn directory(&self, path: &str) -> io::Result<DirectoryHandle> {
        let names = protocol::child_names(&self.logical, path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?;
        let mut directory = self.directory.clone();
        for name in names {
            directory = directory.open_child(OsStr::new(&name))?;
        }
        Ok(directory)
    }

    pub(super) fn read(&self, request: &ReadRequest) -> io::Result<File> {
        let _backup = BackupRead::enable()?;
        let mut names = protocol::child_names(&self.logical, &request.path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?;
        if matches!(request.kind, ReadKind::Directory) {
            let directory = self.directory(&request.path)?;
            // A new handle preserves independent enumeration positions.
            return super::super::read::open_direct(
                directory.path(),
                ReadKind::Directory,
                windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE,
            );
        }
        let Some(name) = names.pop() else {
            return if matches!(request.kind, ReadKind::Metadata) {
                self.directory.file().try_clone()
            } else {
                Err(io::Error::from(io::ErrorKind::InvalidInput))
            };
        };
        let mut parent = self.directory.clone();
        for name in names {
            parent = parent.open_child(OsStr::new(&name))?;
        }
        if matches!(request.kind, ReadKind::File) {
            parent.open_regular_child(OsStr::new(&name))
        } else {
            let file = parent.open_entry(OsStr::new(&name), ReadKind::Metadata)?;
            let class = super::super::directory::classify_open_file(&file)?;
            if class.link_like || class.special {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            Ok(file)
        }
    }

    pub(super) fn send_pins(
        &self,
        request: &ReadRequest,
        pipe: &Pipe,
        parent: HANDLE,
    ) -> io::Result<()> {
        let result = (|| {
            let _backup = BackupRead::enable()?;
            let directory = self.directory(&request.path)?;
            let files = if matches!(request.kind, ReadKind::PinRoot) {
                directory.pin_files()
            } else {
                vec![directory.file()]
            };
            let mut handles = Vec::new();
            for file in files {
                match duplicate_file(file, parent) {
                    Ok(handle) => handles.push(handle),
                    Err(error) => {
                        close_remote(&handles, parent);
                        return Err(error);
                    }
                }
            }
            Ok(PinReply {
                handles,
                path: directory.path().as_os_str().encode_wide().collect(),
                error: None,
            })
        })();
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => PinReply {
                handles: Vec::new(),
                path: Vec::new(),
                error: Some(error_code(&error)),
            },
        };
        if let Err(error) = pipe.send(&reply, parent) {
            // An atomic failed pipe send did not hand duplicates to the client.
            close_remote(&reply.handles, parent);
            return Err(error);
        }
        Ok(())
    }
}

fn close_remote(handles: &[u64], parent: HANDLE) {
    for &handle in handles {
        unsafe {
            DuplicateHandle(
                parent,
                handle as usize as HANDLE,
                null_mut(),
                null_mut(),
                0,
                0,
                DUPLICATE_CLOSE_SOURCE,
            );
        }
    }
}
