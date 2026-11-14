//! Stream priority of the host's download data (plan 2026-11-14 C2 E4,
//! package P2b). The client gives the transfer streams it opens priority -1
//! (`transport_purpose::prioritize_transfer`, `node_bulk.rs`); for a
//! download the host writes the file data, so the send half the host
//! accepted gets the same priority. Replies to navigation, listing, status
//! and every other request keep priority 0, so they never wait behind file
//! data of a running download.
use iroh::endpoint::SendStream;

use crate::share::keepalive::transport_purpose;
use crate::share::wire::FsRequest;

/// Whether the host answers `request` with file data: the requests a client
/// sends on a transfer stream and that carry content from host to client.
/// Uploads and batches of writes reply with a few bytes of status; they stay
/// at the stream default.
pub(super) fn streams_file_data(request: &FsRequest) -> bool {
    matches!(
        request,
        FsRequest::Read { .. } | FsRequest::ReadAt { .. } | FsRequest::GetBatch { .. }
    )
}

/// Gives `send` the transfer priority when `request` is a download; every
/// other request leaves it at 0. Returns whether the request is a download.
/// A stream that is already closed keeps none; its next write reports it.
pub(super) fn prioritize_download(send: &SendStream, request: &FsRequest) -> bool {
    let download = streams_file_data(request);
    if download {
        transport_purpose::prioritize_transfer(send);
    }
    download
}

#[cfg(test)]
#[path = "server_priority_tests.rs"]
mod tests;
