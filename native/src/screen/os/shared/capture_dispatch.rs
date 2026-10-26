//! Child resource control has a dedicated stdin dispatcher. An OS callback can
//! request credit while the capture worker is busy, without borrowing its ask.
use crate::screen::core::{
    capture_admission::{
        CaptureAuthority, CaptureGrant, CapturePlan, PreparedSourceOwnerId, SourceKey,
    },
    capture_grant::DelegatedCaptureCredit,
    capture_import::{CaptureImportGrant, CaptureImportNeed},
    capture_transport,
    worker_capture_wire::{
        self as wire, CreditWire, GrantReply, GrantRequest, GrantValue, ImportNeedWire, PlanWire,
        Reduction,
    },
    worker_wire::{self, WorkerRequest},
};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

pub(super) type Output = Arc<Mutex<Box<dyn Write + Send>>>;
type GrantAnswer = io::Result<GrantValue>;
struct Pending {
    source: String,
    registration: bool,
    answer: mpsc::Sender<GrantAnswer>,
}
struct State {
    next: u64,
    pending: HashMap<u64, Pending>,
    books: HashMap<usize, u64>,
    connections: HashMap<String, u64>,
    allowed: HashSet<String>,
    manual: HashSet<String>,
    owners: HashMap<String, Vec<PreparedSourceOwnerId>>,
    credits: HashSet<u64>,
    ended: bool,
}
pub(super) struct RemoteAuthority {
    session: Mutex<Option<String>>,
    root: Mutex<Option<String>>,
    selection: Mutex<Option<crate::screen::core::window_list::SourceId>>,
    output: Output,
    state: Arc<Mutex<State>>,
}
impl RemoteAuthority {
    pub(super) fn new(output: Output) -> Arc<Self> {
        Arc::new(Self {
            session: Mutex::new(None),
            root: Mutex::new(None),
            selection: Mutex::new(None),
            output,
            state: Arc::new(Mutex::new(State {
                next: 1,
                pending: HashMap::new(),
                books: HashMap::new(),
                connections: HashMap::new(),
                allowed: HashSet::new(),
                manual: HashSet::new(),
                owners: HashMap::new(),
                credits: HashSet::new(),
                ended: false,
            })),
        })
    }
    pub(super) fn bind_session(&self, session: &str) -> io::Result<()> {
        let mut bound = self
            .session
            .lock()
            .map_err(|_| io::Error::other("worker dispatcher poisoned"))?;
        if bound.as_ref().is_some_and(|bound| bound != session) {
            return Err(io::Error::other("worker dispatcher session changed"));
        }
        *bound = Some(session.into());
        Ok(())
    }
    pub(super) fn bind_source(&self, source: &SourceKey) -> io::Result<()> {
        let mut root = self
            .root
            .lock()
            .map_err(|_| io::Error::other("worker dispatcher poisoned"))?;
        if root.as_ref().is_some_and(|root| root != source.source()) {
            return Err(io::Error::other("worker preparation root changed"));
        }
        *root = Some(source.source().into());
        Ok(())
    }
    pub(super) fn bind_selection(
        &self,
        source: crate::screen::core::window_list::SourceId,
    ) -> io::Result<()> {
        let mut selection = self
            .selection
            .lock()
            .map_err(|_| io::Error::other("worker dispatcher poisoned"))?;
        if selection
            .as_ref()
            .is_some_and(|selected| selected != &source)
        {
            return Err(io::Error::other("worker source selection changed"));
        }
        *selection = Some(source);
        Ok(())
    }
    pub(super) fn import(&self, plan: CapturePlan, credit: CreditWire) -> io::Result<CaptureGrant> {
        let session = self
            .session
            .lock()
            .map_err(|_| io::Error::other("worker dispatcher poisoned"))?
            .clone()
            .ok_or_else(|| io::Error::other("worker dispatcher session is unbound"))?;
        let source = SourceKey::new(&session, &credit.source)?;
        if plan.source() != &source || plan.generation().value() != credit.generation {
            return Err(io::Error::other(
                "worker credit identity differs from prepared source",
            ));
        }
        let id = credit.id;
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| io::Error::other("worker dispatcher poisoned"))?;
            if id == 0 || state.credits.contains(&id) {
                return Err(io::Error::other(
                    "worker capture credit was already consumed",
                ));
            }
            state
                .credits
                .try_reserve(1)
                .map_err(|_| io::Error::other("worker credit identity allocation failed"))?;
            state.credits.insert(id);
        }
        let retired = Retirement {
            id,
            output: self.output.clone(),
            state: self.state.clone(),
        };
        let output = self.output.clone();
        let observer = Arc::new(move |owners| {
            let message = Reduction { id, owners };
            if let Ok(mut writer) = output.lock() {
                if let Ok(payload) = crate::screen::core::wire::to_json(&message) {
                    let _ = worker_wire::write_message(&mut *writer, wire::REDUCED, &payload);
                }
            }
        });
        let credit = DelegatedCaptureCredit::from_verified_parent_observed(
            source,
            crate::screen::core::capture_admission::CaptureGeneration::new(credit.generation),
            credit.total,
            credit.owners,
            retired,
            observer,
        )?;
        let grant = CaptureGrant::from_delegated(plan, credit)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("worker dispatcher poisoned"))?;
        state
            .books
            .try_reserve(1)
            .map_err(|_| io::Error::other("worker credit registry allocation failed"))?;
        if state.books.insert(grant.book_identity(), id).is_some() {
            return Err(io::Error::other("worker credit book imported twice"));
        }
        Ok(grant)
    }
    pub(super) fn id_for(&self, grant: &CaptureGrant) -> io::Result<u64> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("worker dispatcher poisoned"))?
            .books
            .get(&grant.book_identity())
            .copied()
            .ok_or_else(|| io::Error::other("worker picture has no imported grant ID"))
    }
    pub(super) fn is_closed(&self) -> bool {
        self.state.lock().map_or(true, |state| state.ended)
    }
    fn deliver(&self, reply: GrantReply) {
        let pending = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let pending = state.pending.remove(&reply.request);
            if let Some(pending) = &pending {
                if pending.registration && !state.ended {
                    match &reply.value {
                        Some(GrantValue::Registered) => {
                            state.allowed.insert(pending.source.clone());
                            state.manual.insert(pending.source.clone());
                        }
                        Some(GrantValue::PreparedOwner { source, owner })
                            if source == &pending.source =>
                        {
                            if let Ok(id) = PreparedSourceOwnerId::new(*owner) {
                                state.owners.entry(source.clone()).or_default().push(id);
                                state.allowed.insert(source.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            pending
        };
        let answer = reply.value.ok_or_else(|| {
            io::Error::new(
                if reply.retryable {
                    io::ErrorKind::WouldBlock
                } else {
                    io::ErrorKind::Other
                },
                reply
                    .error
                    .unwrap_or_else(|| "Parent refused capture credit".into()),
            )
        });
        if let Some(pending) = pending {
            if let Err(mpsc::SendError(Ok(value))) = pending.answer.send(answer) {
                self.release_value(value);
            }
        } else if let Ok(value) = answer {
            self.release_value(value);
        }
    }
    fn release_value(&self, value: GrantValue) {
        match value {
            GrantValue::Frame(credit)
            | GrantValue::Supplement(credit)
            | GrantValue::Import(credit) => self.release_unused(credit.id),
            GrantValue::PreparedOwner { source, owner } => {
                let session = self
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                if let (Some(session), Ok(owner)) = (session, PreparedSourceOwnerId::new(owner)) {
                    if let Ok(source) = SourceKey::new(&session, &source) {
                        self.release_prepared_source(&source, owner);
                    }
                }
            }
            _ => {}
        }
    }
    fn release_unused(&self, id: u64) {
        if let Ok(payload) = crate::screen::core::wire::to_json(&id) {
            if let Ok(mut output) = self.output.lock() {
                let _ = worker_wire::write_message(&mut *output, wire::RETIRED, &payload);
            }
        }
    }
    fn disconnected(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.ended = true;
        state.allowed.clear();
        for (_, pending) in state.pending.drain() {
            let _ = pending.answer.send(Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Parent resource channel closed",
            )));
        }
    }
}

struct Retirement {
    id: u64,
    output: Output,
    state: Arc<Mutex<State>>,
}
impl Drop for Retirement {
    fn drop(&mut self) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.books.retain(|_, id| *id != self.id);
            state.credits.remove(&self.id);
        }
        if let Ok(payload) = crate::screen::core::wire::to_json(&self.id) {
            if let Ok(mut output) = self.output.lock() {
                let _ = worker_wire::write_message(&mut *output, wire::RETIRED, &payload);
            }
        }
    }
}
pub(super) fn start(
    mut input: impl Read + Send + 'static,
    authority: Arc<RemoteAuthority>,
) -> io::Result<mpsc::Receiver<io::Result<WorkerRequest>>> {
    let (sender, requests) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("screen-worker-control".into())
        .spawn(move || {
            loop {
                let message = (|| {
                    let Some((tag, length)) = capture_transport::read_header(&mut input)? else {
                        return Ok(None);
                    };
                    let payload = capture_transport::read_control(&mut input, length)?;
                    if tag == wire::GRANT_REPLY {
                        authority.deliver(crate::screen::core::wire::from_json(&payload)?);
                        return Ok(Some(None));
                    }
                    Ok(Some(Some(WorkerRequest::decode(tag, &payload)?)))
                })();
                match message {
                    Ok(Some(None)) => continue,
                    Ok(Some(Some(request))) => {
                        let close = matches!(request, WorkerRequest::Close);
                        if close {
                            authority.disconnected();
                        }
                        if sender.send(Ok(request)).is_err() || close {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
            authority.disconnected();
        })?;
    Ok(requests)
}

#[path = "capture_dispatch_requests.rs"]
mod requests;

#[cfg(test)]
mod supplement_tests {
    use super::*;
    use crate::screen::core::{
        capture_admission::{CaptureGeneration, CaptureOwner},
        capture_plan::CapturePhases,
    };
    struct Framed {
        bytes: Vec<u8>,
        frames: mpsc::Sender<Vec<u8>>,
    }
    impl Write for Framed {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            let _ = self.frames.send(std::mem::take(&mut self.bytes));
            Ok(())
        }
    }
    #[test]
    fn batches_task_capture_integration_remote_member_supplement_uses_the_image_book() {
        let (frames, written) = mpsc::channel();
        let output: Output = Arc::new(Mutex::new(Box::new(Framed {
            bytes: Vec::new(),
            frames,
        })));
        let remote = RemoteAuthority::new(output);
        remote.bind_session("capture-fixture").unwrap();
        let member = SourceKey::new("capture-fixture", "x11-member").unwrap();
        let root_key = SourceKey::new("capture-fixture", "family").unwrap();
        let mut plan = CapturePlan::new(root_key, CaptureGeneration::new(2));
        let mut child = CapturePlan::new(member.clone(), CaptureGeneration::new(4));
        child
            .add_bytes(CaptureOwner::SourcePixels, 8, 2, CapturePhases::ALL)
            .unwrap();
        plan.add_member(child).unwrap();
        let credit = CreditWire {
            id: 23,
            source: plan.source().source().into(),
            generation: 2,
            total: plan.peak_bytes(),
            owners: plan.owner_limits(),
        };
        let root = remote.import(plan, credit).unwrap();
        let image = root.for_member(&member, CaptureGeneration::new(4)).unwrap();
        remote
            .state
            .lock()
            .unwrap()
            .allowed
            .insert(member.source().into());
        let sending = remote.clone();
        let capture = std::thread::spawn(move || {
            let mut plan = CapturePlan::new(member, CaptureGeneration::new(4));
            plan.add_bytes(CaptureOwner::Readback, 64, 1, CapturePhases::ALL)
                .unwrap();
            plan.add_bytes(CaptureOwner::Metadata, 8, 1, CapturePhases::ALL)
                .unwrap();
            sending.approve_supplement(&image, plan)
        });
        let mut input = io::Cursor::new(written.recv_timeout(Duration::from_secs(2)).unwrap());
        let (tag, length) = capture_transport::read_header(&mut input).unwrap().unwrap();
        assert_eq!(tag, wire::GRANT_REQUEST);
        let payload = capture_transport::read_control(&mut input, length).unwrap();
        let GrantRequest::SupplementPlan {
            request,
            image,
            plan,
        } = crate::screen::core::wire::from_json(&payload).unwrap()
        else {
            panic!("cursor was sent as a replacement frame plan");
        };
        assert_eq!(image, 23);
        let plan = plan.plan("capture-fixture").unwrap();
        assert_eq!(plan.source().source(), "x11-member");
        assert_eq!(plan.generation().value(), 4);
        remote.deliver(GrantReply {
            request,
            value: Some(GrantValue::Supplement(CreditWire {
                id: 24,
                source: plan.source().source().into(),
                generation: 4,
                total: plan.peak_bytes(),
                owners: plan.owner_limits(),
            })),
            error: None,
            retryable: false,
        });
        let cursor = capture.join().unwrap().unwrap();
        assert_eq!(remote.id_for(&root).unwrap(), 23);
        assert_eq!(remote.id_for(&cursor).unwrap(), 24);
        assert!(!root.is_retired().unwrap());
        drop(cursor);
        let mut input = io::Cursor::new(written.recv_timeout(Duration::from_secs(2)).unwrap());
        let (tag, length) = capture_transport::read_header(&mut input).unwrap().unwrap();
        assert_eq!(tag, wire::RETIRED);
        assert_eq!(
            crate::screen::core::wire::from_json::<u64>(
                &capture_transport::read_control(&mut input, length).unwrap()
            )
            .unwrap(),
            24
        );
    }
}
