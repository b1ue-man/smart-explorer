//! One negotiated mutation; reply loss never triggers replay or stage cleanup.
use crate::share::{
    backend::PeerBackend,
    framing::decode_resp,
    fs_access::reversible_replace::validate,
    fs_request::FsReversibleReplace,
    io_deadline, peer_request, peer_stream, peer_telemetry,
    wire::{FsRequest, FsResponse},
};
use std::{io, time::Instant};

pub(super) fn client(
    backend: &PeerBackend,
    staged: &str,
    destination: &str,
    retained: &str,
) -> io::Result<bool> {
    let request = FsReversibleReplace {
        staged: staged.into(),
        destination: destination.into(),
        retained: retained.into(),
    };
    validate(&request)?;
    let supported = peer_stream::features(backend, destination)?.reversible_replace_v1;
    negotiated(
        &request,
        supported,
        backend.owns_stage(staged),
        |request| call_once(backend, request),
        || backend.release_stage(staged),
    )
}

fn negotiated(
    request: &FsReversibleReplace,
    supported: bool,
    owns_stage: bool,
    execute: impl FnOnce(FsRequest) -> io::Result<FsResponse>,
    release: impl FnOnce(),
) -> io::Result<bool> {
    validate(request)?;
    if !supported {
        return Ok(false);
    }
    if !owns_stage {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Stage wurde nicht von diesem Backend angelegt",
        ));
    }
    match execute(FsRequest::ReplaceStagedReversible(request.clone()))? {
        FsResponse::ReversibleReplaced { replaced: true } => {
            release();
            Ok(true)
        }
        FsResponse::ReversibleReplaced { replaced: false } => Ok(false),
        _ => Err(peer_stream::invalid(
            "Unerwartete reversible Ersetzungsantwort",
        )),
    }
}

/// Bypass Attempts entirely, including its extra Idle-close retry. Opening
/// may establish a transport, but this operation frame is sent exactly once.
fn call_once(backend: &PeerBackend, request: FsRequest) -> io::Result<FsResponse> {
    let started = Instant::now();
    let lease = backend.mount_lease_token()?;
    let endpoint = backend.current_endpoint()?;
    let opened = backend.node.open_stream_until(
        &endpoint,
        &backend.identity,
        Instant::now() + peer_request::CONTROL_ATTEMPT_TIMEOUT,
    )?;
    let response = decode_resp(backend.request_once(
        opened,
        request,
        lease,
        Instant::now() + io_deadline::PEER_OP_TIMEOUT,
    )?)?;
    peer_telemetry::report_fs_success(
        &backend.node.ev,
        "replace_staged_reversible",
        started,
        &response,
    );
    Ok(response)
}

#[cfg(test)]
#[path = "reversible_replace_task_tests.rs"]
mod task_tests;
