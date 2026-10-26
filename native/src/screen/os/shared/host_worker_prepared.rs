//! Parent metadata proxy. PREPARE/PLAN never open the child's pixel producer;
//! ACTIVATE sends its exact disjoint credit only after Parent funding succeeds.
use super::*;
use crate::screen::core::{
    capture_admission::{CaptureGrant, SessionAdmission, SourceKey},
    capture_source_contract::{PreparedSource, PreparedSourceInfo},
    host_backend::ScreenInputSink,
    host_codec_adapters::HostOutputPlan,
    worker_capture_wire::{CaptureReply, CaptureRequest, OutputWire, PlanWire, PreparedInfoWire},
};

pub(super) struct PreparedWorker {
    link: SharedLink,
    key: SourceKey,
    info: Mutex<PreparedInfoWire>,
    planned: Mutex<Option<PlanWire>>,
    broker: Arc<capture_broker::CaptureBroker>,
}
impl PreparedWorker {
    pub(super) fn new(
        program: &Path,
        env: &[(String, String)],
        owner: &str,
        session: &str,
        source: &SourceId,
        wanted: Option<FrameSize>,
        control: bool,
        admission: Arc<SessionAdmission>,
        family: Option<crate::screen::core::worker_family::OpenFamily>,
    ) -> Result<Self, SourceRefusal> {
        let make = || -> io::Result<Self> {
            if admission.session() != session {
                return Err(io::Error::other("worker preparation session mismatch"));
            }
            let selected = SourceKey::new(session, &source.to_string())?;
            let broker = capture_broker::CaptureBroker::new(admission, selected);
            let mut link = WorkerLink::start_prepared(program, env, broker.clone())?;
            let request = CaptureRequest::Prepare {
                owner: owner.into(),
                session: session.into(),
                source: source.clone(),
                wanted,
                control,
                working: broker.working_bytes(),
                family,
            };
            let info = match link
                .ask(&WorkerRequest::CaptureControl(request), OPEN_TIMEOUT)
                .map_err(|failure| failure.error)?
            {
                WorkerReply::CaptureControl(CaptureReply::Prepared(info)) => info,
                WorkerReply::Failed { message, .. } | WorkerReply::Refused { reason: message } => {
                    return Err(io::Error::other(message))
                }
                _ => return Err(io::Error::other("unexpected worker preparation reply")),
            };
            let key = SourceKey::new(session, &info.source)?;
            broker.bind_source(key.clone())?;
            info.info()?;
            *link
                .facts
                .limits
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = info.limits;
            link.facts.regions.store(info.regions, Ordering::Release);
            Ok(Self {
                link: Arc::new(Mutex::new(link)),
                key,
                info: Mutex::new(info),
                planned: Mutex::new(None),
                broker,
            })
        };
        make().map_err(|error| SourceRefusal::new(&format!("host_process: {error}")))
    }
    pub(super) fn shared_link(&self) -> SharedLink {
        self.link.clone()
    }
}
impl PreparedSource for PreparedWorker {
    fn source_key(&self) -> &SourceKey {
        &self.key
    }
    fn description(&self) -> PreparedSourceInfo {
        let info = self.info.lock().unwrap_or_else(PoisonError::into_inner);
        PreparedSourceInfo {
            native_size: info.size,
            capture_limits: info.limits,
            supports_regions: info.regions,
            state: info.state,
            generation: crate::screen::core::capture_admission::CaptureGeneration::new(
                info.generation,
            ),
        }
    }
    fn input_sink(&self) -> Option<Arc<dyn ScreenInputSink>> {
        let facts = self
            .link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .facts
            .clone();
        Some(source::input_sink(self.link.clone(), facts))
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        match self
            .link
            .lock()
            .map_err(|_| io::Error::other("worker link poisoned"))?
            .ask(
                &WorkerRequest::CaptureControl(CaptureRequest::Activity(active)),
                FRAME_TIMEOUT,
            )
            .map_err(|failure| failure.error)?
        {
            WorkerReply::CaptureControl(CaptureReply::Activity(value)) if value == active => Ok(()),
            WorkerReply::Failed { message, .. } => Err(io::Error::other(message)),
            _ => Err(io::Error::other(
                "worker did not acknowledge prepared activity",
            )),
        }
    }
    fn capture_plan(
        &self,
        output: &HostOutputPlan,
    ) -> io::Result<Option<crate::screen::core::capture_admission::CapturePlan>> {
        self.broker.configure(output.clone())?;
        let reply = self
            .link
            .lock()
            .map_err(|_| io::Error::other("worker link poisoned"))?
            .ask(
                &WorkerRequest::CaptureControl(CaptureRequest::Plan(OutputWire::from_plan(output))),
                OPEN_TIMEOUT,
            )
            .map_err(|failure| failure.error)?;
        let (info, plan) = match reply {
            WorkerReply::CaptureControl(CaptureReply::Plan { info, plan }) => (info, plan),
            WorkerReply::Failed { message, .. } => return Err(io::Error::other(message)),
            _ => return Err(io::Error::other("unexpected worker plan reply")),
        };
        if info.source != self.key.source() {
            return Err(io::Error::other("worker plan changed its source"));
        }
        info.info()?;
        let facts = self
            .link
            .lock()
            .map_err(|_| io::Error::other("worker link poisoned"))?
            .facts
            .clone();
        *facts
            .limits
            .lock()
            .map_err(|_| io::Error::other("worker limits poisoned"))? = info.limits;
        facts.regions.store(info.regions, Ordering::Release);
        *self
            .info
            .lock()
            .map_err(|_| io::Error::other("worker metadata poisoned"))? = info;
        let Some(plan) = plan else {
            *self
                .planned
                .lock()
                .map_err(|_| io::Error::other("worker plan poisoned"))? = None;
            return Ok(None);
        };
        let worker = plan.plan(self.key.session())?;
        if worker.source() != &self.key {
            return Err(io::Error::other("worker plan has another source identity"));
        }
        let parent = self.broker.parent_plan(&worker)?;
        *self
            .planned
            .lock()
            .map_err(|_| io::Error::other("worker plan poisoned"))? = Some(plan);
        Ok(Some(parent))
    }
    fn activate(
        self: Box<Self>,
        parent: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let activate = || -> io::Result<Box<dyn ScreenSource>> {
            let planned = self
                .planned
                .lock()
                .map_err(|_| io::Error::other("worker plan poisoned"))?
                .take()
                .ok_or_else(|| io::Error::other("worker pixel metadata is pending"))?;
            let worker = planned.plan(self.key.session())?;
            let credit = self.broker.approve(worker, Some(parent))?;
            let mut link = self
                .link
                .lock()
                .map_err(|_| io::Error::other("worker link poisoned"))?;
            match link
                .ask(
                    &WorkerRequest::CaptureControl(CaptureRequest::Activate { credit }),
                    OPEN_TIMEOUT,
                )
                .map_err(|failure| failure.error)?
            {
                WorkerReply::CaptureControl(CaptureReply::Activated) => {}
                WorkerReply::Failed { message, .. } | WorkerReply::Refused { reason: message } => {
                    return Err(io::Error::other(message))
                }
                _ => {
                    return Err(io::Error::other(
                        "worker did not acknowledge grant activation",
                    ))
                }
            }
            let size = self
                .description()
                .native_size
                .ok_or_else(|| io::Error::other("activated worker still has unknown pixel size"))?;
            Ok(Box::new(WorkerSource {
                facts: link.facts.clone(),
                broker: self.broker.clone(),
                link: self.link.clone(),
                size,
            }))
        };
        activate().map_err(|error| SourceRefusal::new(&error.to_string()))
    }
}
impl super::backend::WorkerBackend {
    pub(super) fn prepare_worker(
        &self,
        owner: &str,
        session: &str,
        source: &SourceId,
        wanted: Option<FrameSize>,
        control: bool,
        admission: Arc<SessionAdmission>,
    ) -> Result<Box<dyn PreparedSource>, SourceRefusal> {
        #[cfg(test)]
        if self.program == super::backend::WorkerProgram::InProcess {
            return self
                .inner
                .prepare_session_source(owner, session, source, wanted, control, admission);
        }
        let (program, env): (Option<&Path>, &[(String, String)]) = match &self.program {
            super::backend::WorkerProgram::Installation { gui, current } => {
                (program_for(gui.as_deref(), current.as_deref(), source), &[])
            }
            #[cfg(test)]
            super::backend::WorkerProgram::Fixed { path, env } => {
                (Some(path.as_path()), env.as_slice())
            }
            #[cfg(test)]
            super::backend::WorkerProgram::InProcess => {
                return Err(SourceRefusal::new(
                    "inprocess preparation was not delegated",
                ))
            }
        };
        let program = program
            .ok_or_else(|| SourceRefusal::new("host_process: desktop program unavailable"))?;
        let prepared = if crate::screen::core::family_protocol::selector(source).is_some() {
            let pin = self
                .families
                .lock()
                .map_err(|_| SourceRefusal::new("family pin store poisoned"))?
                .take(owner, source)
                .map_err(|error| SourceRefusal::new(&error.to_string()))?;
            pin.prepare(
                program, env, owner, session, source, wanted, control, admission,
            )?
        } else {
            PreparedWorker::new(
                program, env, owner, session, source, wanted, control, admission, None,
            )?
        };
        if control {
            let mut controls = self.controls.lock().unwrap_or_else(PoisonError::into_inner);
            controls.retain(|link| link.strong_count() > 0);
            controls.push(Arc::downgrade(&prepared.shared_link()));
        }
        Ok(Box::new(prepared))
    }
}
