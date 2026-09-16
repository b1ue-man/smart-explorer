use super::prelude::*;
use super::*;

enum UploadSelection {
    Paths(Vec<String>),
    Filtered(Vec<(String, String)>),
}

impl App {
    /// Upload local files/folders. Shared by clipboard and drag/drop routing.
    pub(in crate::app) fn start_remote_upload(
        &mut self,
        paths: Vec<String>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        self.start_clipboard_upload(UploadSelection::Paths(paths), backend, dest_root);
    }

    /// Preserve the filtered clipboard's relative paths rather than flattening
    /// each absolute source path into the destination directory.
    pub(in crate::app) fn start_filtered_remote_upload(
        &mut self,
        pairs: Vec<(String, String)>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        self.start_clipboard_upload(UploadSelection::Filtered(pairs), backend, dest_root);
    }

    fn start_clipboard_upload(
        &mut self,
        selection: UploadSelection,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        let request = match selection {
            UploadSelection::Paths(paths) => super::transfer_jobs::TransferRequest::Upload {
                paths,
                backend,
                dest_root,
            },
            UploadSelection::Filtered(pairs) => {
                super::transfer_jobs::TransferRequest::UploadPairs {
                    pairs,
                    backend,
                    dest_root,
                }
            }
        };
        self.submit_transfer(request);
    }
}
