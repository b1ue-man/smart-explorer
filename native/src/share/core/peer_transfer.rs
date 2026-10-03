//! Client side of transfer v1: the host's advertised features (probed once
//! per connection), the connection's flow identity and transfer slots, this
//! client's own copy stages, and single-file requests that save round trips.
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::share::core::eio;
use crate::share::io_deadline::PEER_OP_TIMEOUT;
use crate::share::keepalive::TRANSFER_STREAMS_PER_CONNECTION;
use crate::share::node_sessions::OpenedPeerStream;
use crate::share::peer_request::{CONTROL_ATTEMPT_TIMEOUT, IDEMPOTENT_CONTROL_BUDGET};
use crate::share::wire::{
    discardable_stage, plan_batches, BatchPart, Ctrl, FsRequest, FsResponse,
    FsTransferCapabilities, BATCH_MAX_BYTES, BATCH_MAX_FILES,
};
use crate::vfs::{Backend, BatchLimits, VfsResult};

use super::PeerBackend;

/// Hosts before transfer v1 run 32 blocking operations at once for all their
/// clients and queue the rest while the client's 60 s deadline runs; more in
/// flight would only wait there.
const LEGACY_HOST_ADMISSION: u32 = 32;

/// Transfers of a host's admission kept for the foreground beside a busy
/// flow: as many as the streams kept for browsing (four, research §2).
const FOREGROUND_TRANSFERS: u32 = crate::share::keepalive::CONTROL_STREAM_RESERVE;

/// After a failed probe the old path is used this long before asking again:
/// two keepalive periods (5 s), in which a broken connection is noticed, so
/// an unreachable host does not cost a probe per file.
const FAILED_PROBE_RETRY: Duration = Duration::from_secs(10);

/// Own stages tracked at once: far above any in-flight window (60 per
/// connection), so only leaked entries could fill it.
const MAX_TRACKED_STAGES: usize = 1 << 16;

/// A host copies at least 1 MiB/s even between slow backends (an SD card, a
/// remote connection on a thin uplink). The live connection (keepalive 5 s,
/// idle timeout 20 s) ends the wait at once if the host goes away, so this
/// generous bound only limits a stuck host.
const MIN_SERVER_COPY_RATE: u64 = 1024 * 1024;

/// Transfer state of one PeerBackend.
#[derive(Default)]
pub(in crate::share) struct PeerTransferState {
    caps: Mutex<Option<CachedCaps>>,
    pub(in crate::share) features:
        Mutex<Option<(Option<usize>, crate::share::wire::FsHostFeatures)>>,
    stages: Mutex<HashSet<String>>,
}

#[derive(Clone, Copy)]
struct CachedCaps {
    caps: FsTransferCapabilities,
    /// Connection the probe answered on; a reconnect may reach a new host.
    generation: Option<usize>,
    probed_at: Instant,
    confirmed: bool,
}

impl PeerBackend {
    /// The host's transfer features, probed once per connection generation.
    pub(super) fn transfer_caps(&self) -> FsTransferCapabilities {
        let generation = self.node.outgoing_generation(self.initial_endpoint());
        if let Some(cached) = self.cached_caps() {
            let fresh = if cached.confirmed {
                generation.is_some() && cached.generation == generation
            } else {
                cached.probed_at.elapsed() < FAILED_PROBE_RETRY
            };
            if fresh {
                return cached.caps;
            }
        }
        let (caps, confirmed) = match self.probe_transfer_caps() {
            Ok(caps) => (caps, true),
            Err(_) => (FsTransferCapabilities::default(), false),
        };
        let cached = CachedCaps {
            caps,
            generation: self.node.outgoing_generation(self.initial_endpoint()),
            probed_at: Instant::now(),
            confirmed,
        };
        if let Ok(mut slot) = self.transfer.caps.lock() {
            *slot = Some(cached);
        }
        caps
    }

    fn cached_caps(&self) -> Option<CachedCaps> {
        self.transfer.caps.lock().ok().and_then(|slot| *slot)
    }

    /// Capabilities of `/` carry the host-wide transfer features without
    /// resolving any export.
    fn probe_transfer_caps(&self) -> io::Result<FsTransferCapabilities> {
        if self.legacy_capabilities()? {
            return Ok(FsTransferCapabilities::default());
        }
        let request = FsRequest::Capabilities {
            path: "/".into(),
            acquire_lease: false,
            lease_request_id: None,
        };
        match self.request_unleased_until(request, Instant::now() + IDEMPOTENT_CONTROL_BUDGET)? {
            FsResponse::Capabilities { capabilities, .. } => Ok(capabilities.transfer),
            _ => Err(eio("unerwartete Antwort auf capabilities")),
        }
    }

    pub(super) fn transfer_v1(&self) -> bool {
        self.transfer_caps().v1
    }

    /// Batch limits both sides accept; `None` before transfer v1.
    fn host_batch_limits(&self) -> Option<BatchLimits> {
        let caps = self.transfer_caps();
        if !caps.v1 || caps.batch_max_files == 0 || caps.batch_max_bytes == 0 {
            return None;
        }
        Some(BatchLimits {
            max_files: caps.batch_max_files.min(BATCH_MAX_FILES) as usize,
            max_bytes: caps.batch_max_bytes.min(BATCH_MAX_BYTES),
        })
    }

    /// Batches for files below `dir`; the synthetic containers hold none.
    pub(super) fn batch_limits_below(&self, dir: &str) -> Option<BatchLimits> {
        if crate::share::fs_capabilities::is_synthetic_container(dir) {
            return None;
        }
        self.host_batch_limits()
    }

    pub(super) fn batch_limits_required(&self) -> io::Result<BatchLimits> {
        self.host_batch_limits().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Die Gegenstelle überträgt keine Pakete",
            )
        })
    }

    /// Transfers the adaptive flow may run at once on this connection: the
    /// host's admission, never more than the streams left beside browsing.
    /// A transfer v1 host refuses anything beyond its admission (`Busy`),
    /// so the flow leaves `FOREGROUND_TRANSFERS` of it to opening files,
    /// previews and mount reads; older hosts queue those instead.
    pub(super) fn transfer_slots(&self) -> usize {
        let caps = self.transfer_caps();
        let admission = if caps.v1 && caps.admission > 0 {
            caps.admission
                .min(TRANSFER_STREAMS_PER_CONNECTION)
                .saturating_sub(FOREGROUND_TRANSFERS)
        } else {
            LEGACY_HOST_ADMISSION.min(TRANSFER_STREAMS_PER_CONNECTION)
        };
        admission.max(1) as usize
    }

    /// Every path of one peer travels over one QUIC connection and so
    /// shares one adaptive flow.
    pub(super) fn connection_flow_key(&self) -> String {
        let session = crate::share::session::session_key(self.initial_endpoint());
        format!("share:{session}")
    }

    /// Remembers a file this backend created exclusively when its name is an
    /// upload stage (engine stages also arrive through the background
    /// service's plain `open_write_new`); only those may be discarded.
    pub(super) fn track_stage(&self, stage: &str) {
        let name = stage.rsplit('/').next().unwrap_or(stage);
        if !discardable_stage(name) {
            return;
        }
        if let Ok(mut stages) = self.transfer.stages.lock() {
            if stages.len() < MAX_TRACKED_STAGES {
                stages.insert(stage.to_string());
            }
        }
    }

    /// The stage was published, renamed or removed: no longer ours to drop.
    pub(in crate::share) fn release_stage(&self, stage: &str) {
        if let Ok(mut stages) = self.transfer.stages.lock() {
            stages.remove(stage);
        }
    }

    pub(in crate::share) fn owns_stage(&self, stage: &str) -> bool {
        self.transfer
            .stages
            .lock()
            .is_ok_and(|stages| stages.contains(stage))
    }

    /// One directory level in one request; before transfer v1 the path-based
    /// fallbacks of the Backend trait (MkdirAll, probe before exclusive).
    pub(super) fn create_directory(&self, path: &str, exclusive: bool) -> VfsResult<()> {
        if self.transfer_v1() {
            let request = FsRequest::CreateDir {
                path: path.to_string(),
                exclusive,
            };
            return expect_ok(self.request(request)?, "create_dir");
        }
        if exclusive && self.try_exists(path)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                path.to_string(),
            ));
        }
        self.mkdir_all(path)
    }

    /// Publishes a stage without replacing: one request on a transfer v1 host
    /// (it validates the stage itself), else a stat and a no-replace rename.
    pub(super) fn promote_without_replace(
        &self,
        staged: &str,
        destination: &str,
        copy: bool,
    ) -> VfsResult<()> {
        let result = if self.transfer_v1() {
            let request = FsRequest::PromoteNoReplace {
                staged: staged.to_string(),
                destination: destination.to_string(),
                copy,
            };
            self.request(request)
                .and_then(|response| expect_ok(response, "promote_no_replace"))
        } else {
            crate::vfs::promote_staged_no_replace_with(self, staged, destination, |from, to| {
                self.rename_no_replace(from, to)
            })
        };
        if result.is_ok() {
            self.release_stage(staged);
        }
        result
    }

    /// Removes a stage this backend created and never published; any other
    /// path, or a host before transfer v1, stays `Unsupported` (K17).
    pub(super) fn discard_own_stage(&self, stage: &str) -> VfsResult<()> {
        if !self.owns_stage(stage) || !self.transfer_v1() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Nur eigene, unveröffentlichte Stufen werden entfernt",
            ));
        }
        let request = FsRequest::DiscardStage {
            path: stage.to_string(),
        };
        let result = self
            .request(request)
            .and_then(|response| expect_ok(response, "discard_stage"));
        match &result {
            Ok(()) => self.release_stage(stage),
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.release_stage(stage),
            Err(_) => {}
        }
        result
    }

    /// Host-side copy into a new private stage (CopyFile exists on every
    /// host); the reply waits for the whole copy, so its deadline grows with
    /// the size.
    pub(super) fn copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<u64>> {
        self.track_stage(stage);
        let request = FsRequest::CopyFile {
            src: src.to_string(),
            dst: stage.to_string(),
        };
        match self.request_with_budget(request, server_copy_budget(size))? {
            FsResponse::Data { size } => Ok(Some(size)),
            FsResponse::Ok => self.stat(stage).map(|metadata| Some(metadata.size)),
            _ => Err(eio("unerwartete Antwort auf copy_file")),
        }
    }

    /// Reads from `offset` to resume; `None` where the host cannot.
    pub(super) fn read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        if !self.transfer_v1() {
            return Ok(None);
        }
        let request = FsRequest::ReadAt {
            path: path.to_string(),
            id: id.map(str::to_string),
            offset,
        };
        match self.open_read_request(request, "peer read at") {
            Ok(reader) => Ok(Some(reader)),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Reads one of several equal names by its provider ID where the host
    /// understands it; otherwise by path, as before.
    pub(super) fn read_by_id(
        &self,
        path: &str,
        id: Option<&str>,
    ) -> VfsResult<Box<dyn Read + Send>> {
        if id.is_some() {
            if let Some(reader) = self.read_at(path, id, 0)? {
                return Ok(reader);
            }
        }
        self.open_reader(path)
    }

    /// A new stream for one batch.
    pub(super) fn open_batch_stream(&self) -> io::Result<OpenedPeerStream> {
        let endpoint = self.current_endpoint()?;
        self.node.open_stream_until(
            &endpoint,
            &self.identity,
            Instant::now() + CONTROL_ATTEMPT_TIMEOUT,
        )
    }
}

fn expect_ok(response: FsResponse, operation: &str) -> io::Result<()> {
    match response {
        FsResponse::Ok => Ok(()),
        _ => Err(eio(format!("unerwartete Antwort auf {operation}"))),
    }
}

fn server_copy_budget(size: u64) -> Duration {
    PEER_OP_TIMEOUT.saturating_add(Duration::from_secs(size / MIN_SERVER_COPY_RATE))
}

/// Splits a batch request by the host's limits and by its encoded header
/// (K18a); `envelope` is the request with an empty list.
pub(super) fn plan_request<T: Serialize>(
    envelope: &Ctrl,
    items: &[T],
    size_of: impl Fn(&T) -> u64,
    limits: BatchLimits,
) -> io::Result<Vec<BatchPart>> {
    let envelope = serde_json::to_vec(envelope).map_err(eio)?.len();
    let measured = items
        .iter()
        .map(|item| -> io::Result<(usize, u64)> {
            Ok((serde_json::to_vec(item).map_err(eio)?.len(), size_of(item)))
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(plan_batches(
        envelope,
        &measured,
        limits.max_files,
        limits.max_bytes,
    ))
}

/// A copy of `error` for each entry it applies to; congestion stays
/// congestion, so the flow backs off instead of failing the files.
pub(super) fn copy_error(error: &io::Error) -> io::Error {
    match crate::vfs::congestion_of(error) {
        Some(congestion) => {
            crate::vfs::congestion_error(congestion.message.clone(), congestion.retry_after)
        }
        None => io::Error::new(error.kind(), error.to_string()),
    }
}

/// A stage writer that commits only when exactly `size` bytes came (sized
/// copy stages, W1); dropping it before `flush` aborts the upload.
pub(super) fn sized_writer(inner: Box<dyn Write + Send>, size: u64) -> Box<dyn Write + Send> {
    Box::new(SizedWriter {
        inner,
        expected: size,
        written: 0,
    })
}

struct SizedWriter {
    inner: Box<dyn Write + Send>,
    expected: u64,
    written: u64,
}

impl Write for SizedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written.saturating_add(buf.len() as u64) > self.expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Quelle liefert mehr als die angekündigten {} Bytes",
                    self.expected
                ),
            ));
        }
        let written = self.inner.write(buf)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.written != self.expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{} statt {} Bytes für die Stufe empfangen",
                    self.written, self.expected
                ),
            ));
        }
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder(std::sync::Arc<Mutex<Vec<u8>>>, std::sync::Arc<Mutex<bool>>);

    impl Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            *self.1.lock().unwrap() = true;
            Ok(())
        }
    }

    #[test]
    fn transfer_engine_task_sized_stage_commits_only_exact_length() {
        let bytes = std::sync::Arc::new(Mutex::new(Vec::new()));
        let flushed = std::sync::Arc::new(Mutex::new(false));
        let mut short = sized_writer(Box::new(Recorder(bytes.clone(), flushed.clone())), 5);
        short.write_all(b"abc").unwrap();
        assert_eq!(
            short.flush().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(
            !*flushed.lock().unwrap(),
            "a short stage is never committed"
        );
        assert_eq!(
            short.write(b"def").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        short.write_all(b"de").unwrap();
        short.flush().unwrap();
        assert!(*flushed.lock().unwrap());
        assert_eq!(*bytes.lock().unwrap(), b"abcde");

        let congestion = crate::vfs::congestion_error("voll", Some(Duration::from_secs(2)));
        let copied = copy_error(&congestion);
        let copied = crate::vfs::congestion_of(&copied).expect("congestion stays congestion");
        assert_eq!(copied.retry_after, Some(Duration::from_secs(2)));
        let plain = copy_error(&io::Error::new(io::ErrorKind::NotFound, "weg"));
        assert_eq!(plain.kind(), io::ErrorKind::NotFound);
        assert_eq!(plain.to_string(), "weg");
        assert_eq!(server_copy_budget(0), PEER_OP_TIMEOUT);
        assert_eq!(
            server_copy_budget(10 * MIN_SERVER_COPY_RATE),
            PEER_OP_TIMEOUT + Duration::from_secs(10)
        );
    }
}
