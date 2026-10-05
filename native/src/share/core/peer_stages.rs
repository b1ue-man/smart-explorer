//! Creator tickets are separate from the narrower upload-discard contract.
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

use crate::share::{backend::PeerBackend, session::relation_kind_id};
use crate::vfs::{Backend, VfsMeta};

const MAX_TRACKED_STAGES: usize = 1 << 16;

#[derive(Clone, Default)]
pub(in crate::share) struct StageLedger(Arc<Mutex<Ledger>>);

#[derive(Default)]
struct Ledger {
    serial: u64,
    entries: HashMap<String, Entry>,
}

struct Entry {
    serial: u64,
    binding: Binding,
    phase: Phase,
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::share) struct Binding {
    peer: String,
    lease: Option<String>,
}

impl Binding {
    pub(in crate::share) fn new(peer: String, lease: Option<String>) -> Self {
        Self { peer, lease }
    }
}

enum Phase {
    Creating,
    Writing,
    Ready(Proof),
    Pending(Option<Proof>),
}

impl Phase {
    fn label(&self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Writing => "writing",
            Self::Ready(_) => "ready",
            Self::Pending(_) => "pending",
        }
    }
}

#[derive(Clone)]
struct Proof {
    size: u64,
    snapshot: Option<Snapshot>,
}

#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    id: Option<String>,
    size: u64,
    mtime_ms: i64,
    content_md5: Option<String>,
}

impl Snapshot {
    fn regular(meta: &VfsMeta) -> io::Result<Self> {
        if meta.is_dir || meta.is_symlink || meta.special {
            return Err(denied("Stage ist keine reguläre Datei"));
        }
        Ok(Self {
            id: meta.id.clone(),
            size: meta.size,
            mtime_ms: meta.mtime_ms,
            content_md5: meta.content_md5.clone(),
        })
    }
}

#[derive(Clone)]
pub(in crate::share) struct StageTicket {
    ledger: StageLedger,
    path: String,
    serial: u64,
}

impl StageLedger {
    pub(in crate::share) fn reserve(
        &self,
        path: &str,
        binding: Binding,
    ) -> io::Result<StageTicket> {
        if !client_stage_name(path) {
            return Err(not_own_at(path, "unrecognized-name"));
        }
        let mut ledger = self.0.lock().map_err(|_| poisoned())?;
        if ledger.entries.contains_key(path) {
            return Err(denied("Stagepfad hat bereits ein Creator-Ticket"));
        }
        if ledger.entries.len() >= MAX_TRACKED_STAGES {
            return Err(io::Error::other("Zu viele unveröffentlichte Share-Stufen"));
        }
        ledger.serial = ledger.serial.checked_add(1).ok_or_else(poisoned)?;
        let serial = ledger.serial;
        ledger.entries.insert(
            path.into(),
            Entry {
                serial,
                binding,
                phase: Phase::Creating,
            },
        );
        Ok(StageTicket {
            ledger: self.clone(),
            path: path.into(),
            serial,
        })
    }

    pub(in crate::share) fn contains(&self, path: &str) -> io::Result<bool> {
        Ok(self
            .0
            .lock()
            .map_err(|_| poisoned())?
            .entries
            .contains_key(path))
    }

    pub(in crate::share) fn ready(&self, path: &str, binding: &Binding) -> io::Result<StageTicket> {
        let mut ledger = self.0.lock().map_err(|_| poisoned())?;
        let entry = ledger
            .entries
            .get_mut(path)
            .ok_or_else(|| not_own_at(path, "untracked"))?;
        if &entry.binding != binding {
            entry.phase = Phase::Pending(None);
            return Err(denied("Stage gehört zu einer anderen Share-Freigabe"));
        }
        if !matches!(&entry.phase, Phase::Ready(_)) {
            return Err(not_own_at(path, entry.phase.label()));
        }
        Ok(StageTicket {
            ledger: self.clone(),
            path: path.into(),
            serial: entry.serial,
        })
    }

    pub(in crate::share) fn verify(
        &self,
        path: &str,
        binding: &Binding,
        meta: &VfsMeta,
    ) -> io::Result<StageTicket> {
        let mut ledger = self.0.lock().map_err(|_| poisoned())?;
        let entry = ledger
            .entries
            .get_mut(path)
            .ok_or_else(|| not_own_at(path, "untracked"))?;
        if &entry.binding != binding {
            entry.phase = Phase::Pending(None);
            return Err(denied("Stage gehört zu einer anderen Share-Freigabe"));
        }
        let snapshot = match Snapshot::regular(meta) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                entry.phase = Phase::Pending(None);
                return Err(error);
            }
        };
        let proof = match &mut entry.phase {
            Phase::Ready(proof) => proof,
            phase => return Err(not_own_at(path, phase.label())),
        };
        if snapshot.size != proof.size
            || proof
                .snapshot
                .as_ref()
                .is_some_and(|expected| expected != &snapshot)
        {
            entry.phase = Phase::Pending(None);
            return Err(denied("Stage wurde seit dem Schreibabschluss verändert"));
        }
        // WriteNew/Ready and the successful Writer acknowledgement establish
        // creation. This snapshot only binds subsequent uses to that outcome.
        proof.snapshot = Some(snapshot);
        Ok(StageTicket {
            ledger: self.clone(),
            path: path.into(),
            serial: entry.serial,
        })
    }
}

impl StageTicket {
    fn change(&self, change: impl FnOnce(&mut Phase) -> io::Result<()>) -> io::Result<()> {
        let mut ledger = self.ledger.0.lock().map_err(|_| poisoned())?;
        let entry = ledger.entries.get_mut(&self.path).ok_or_else(not_own)?;
        if entry.serial != self.serial {
            return Err(not_own());
        }
        change(&mut entry.phase)
    }

    pub(in crate::share) fn opened(&self) -> io::Result<()> {
        self.change(|phase| {
            if !matches!(phase, Phase::Creating) {
                return Err(not_own());
            }
            *phase = Phase::Writing;
            Ok(())
        })
    }

    pub(in crate::share) fn committed(&self, size: u64) -> io::Result<()> {
        self.change(|phase| {
            if !matches!(phase, Phase::Writing) {
                return Err(not_own());
            }
            *phase = Phase::Ready(Proof {
                size,
                snapshot: None,
            });
            Ok(())
        })
    }

    pub(in crate::share) fn begin(&self) -> io::Result<()> {
        self.change(|phase| {
            let Phase::Ready(proof) = phase else {
                return Err(not_own());
            };
            *phase = Phase::Pending(Some(proof.clone()));
            Ok(())
        })
    }

    pub(in crate::share) fn uncertain(&self) {
        let _ = self.change(|phase| {
            *phase = Phase::Pending(None);
            Ok(())
        });
    }

    pub(in crate::share) fn unmodified(&self) -> io::Result<()> {
        self.change(|phase| {
            let Phase::Pending(Some(proof)) = phase else {
                return Err(not_own());
            };
            *phase = Phase::Ready(proof.clone());
            Ok(())
        })
    }

    pub(in crate::share) fn finished(&self, meta: &VfsMeta) -> io::Result<()> {
        let current = Snapshot::regular(meta)?;
        self.change(|phase| {
            let Phase::Pending(Some(proof)) = phase else {
                return Err(not_own());
            };
            let before = proof.snapshot.as_ref().ok_or_else(not_own)?;
            if before.id != current.id
                || proof.size != current.size
                || before.content_md5 != current.content_md5
            {
                return Err(denied("Stage wurde während der Metadatenänderung ersetzt"));
            }
            *phase = Phase::Ready(Proof {
                size: proof.size,
                snapshot: Some(current),
            });
            Ok(())
        })
    }

    pub(in crate::share) fn release(&self) {
        if let Ok(mut ledger) = self.ledger.0.lock() {
            if ledger
                .entries
                .get(&self.path)
                .is_some_and(|entry| entry.serial == self.serial)
            {
                ledger.entries.remove(&self.path);
            }
        }
    }
}

impl PeerBackend {
    pub(in crate::share) fn open_owned_writer(
        &self,
        path: &str,
    ) -> io::Result<Box<dyn std::io::Write + Send>> {
        let ticket = self.reserve_stage(path)?;
        let opened = self.open_writer(
            crate::share::wire::FsRequest::WriteNew { path: path.into() },
            "peer exclusive write open",
        );
        match (opened, ticket) {
            (Ok(writer), Some(ticket)) => {
                if let Err(error) = ticket.opened() {
                    ticket.uncertain();
                    return Err(error);
                }
                Ok(crate::share::peer_writer::owned_writer(writer, ticket))
            }
            (Ok(writer), None) => Ok(writer),
            (Err(error), ticket) => {
                if let Some(ticket) = ticket {
                    ticket.uncertain();
                }
                Err(error)
            }
        }
    }

    fn stage_binding(&self) -> io::Result<Binding> {
        let endpoint = self.current_endpoint()?;
        let (kind, relation) = relation_kind_id(&endpoint);
        Ok(Binding::new(
            format!("peer:{kind}:{relation}:{}", endpoint.presence.node_id),
            self.mount_lease_token()?,
        ))
    }

    pub(in crate::share) fn reserve_stage(&self, path: &str) -> io::Result<Option<StageTicket>> {
        if !client_stage_name(path) {
            return Ok(None);
        }
        self.transfer
            .stages
            .reserve(path, self.stage_binding()?)
            .map(Some)
    }

    pub(in crate::share) fn verify_owned_stage(&self, path: &str) -> io::Result<StageTicket> {
        let binding = self.stage_binding()?;
        // Reject open/unconfirmed tickets before a provider stat: a provider
        // may hold its only session in the still-open writer.
        let ticket = self.transfer.stages.ready(path, &binding)?;
        let meta = match self.stat(path) {
            Ok(meta) => meta,
            Err(error) => {
                if error.kind() == io::ErrorKind::NotFound {
                    ticket.uncertain();
                }
                return Err(error);
            }
        };
        if binding != self.stage_binding()? {
            ticket.uncertain();
            return Err(denied(
                "Share-Freigabe hat sich während der Stageprüfung geändert",
            ));
        }
        self.transfer.stages.verify(path, &binding, &meta)
    }

    pub(in crate::share) fn begin_stage_publication(&self, path: &str) -> io::Result<StageTicket> {
        let ticket = self.verify_owned_stage(path)?;
        ticket.begin()?;
        Ok(ticket)
    }

    pub(in crate::share) fn begin_tracked_mutation(
        &self,
        path: &str,
    ) -> io::Result<Option<StageTicket>> {
        if !self.transfer.stages.contains(path)? {
            return Ok(None);
        }
        self.begin_stage_publication(path).map(Some)
    }

    pub(in crate::share) fn stage_request_once(
        &self,
        request: crate::share::wire::FsRequest,
        operation: &'static str,
    ) -> io::Result<crate::share::wire::FsResponse> {
        crate::share::peer_extensions::reversible_replace::call_once(self, request, operation)
    }

    pub(in crate::share) fn tracked_request(
        &self,
        request: crate::share::wire::FsRequest,
        ticket: Option<&StageTicket>,
        operation: &'static str,
    ) -> io::Result<crate::share::wire::FsResponse> {
        if ticket.is_some() {
            self.stage_request_once(request, operation)
        } else {
            self.request(request)
        }
    }
}

fn client_stage_name(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    // Eligibility for exclusive creator tracking, never proof of ownership.
    crate::vfs::is_unique_stage(name)
}

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
fn not_own() -> io::Error {
    denied("Stage wurde nicht von diesem Backend angelegt")
}
fn not_own_at(path: &str, phase: &str) -> io::Error {
    denied(&format!(
        "Stage wurde nicht von diesem Backend angelegt (path={path:?}, state={phase})"
    ))
}
fn poisoned() -> io::Error {
    io::Error::other("Share-Stage-Ledger ist nicht verfügbar")
}

#[cfg(test)]
#[path = "sync_reliability_task_old_jobs_peer_stage_tests.rs"]
mod sync_reliability_task_old_jobs_peer_stage_tests;
