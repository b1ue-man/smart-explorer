//! Broker authority for one authenticated, inherited helper channel.
use crate::{
    login::core::{
        capture_wire::{Admission, GrantReply, Plan},
        wire::Reply,
    },
    screen::core::{
        capture_admission::{CaptureGrant, CaptureOwner, SessionAdmission},
        capture_bytes::AdmittedBytes,
        frame_budget::MAX_WORKING_BYTES,
        frame_memory::{self, CapacityCeiling},
    },
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

struct Entry {
    id: u64,
    descriptor: Plan,
    grant: CaptureGrant,
}
pub(crate) struct ParentCapture {
    admission: Arc<SessionAdmission>,
    entries: Mutex<Vec<Entry>>,
    last_id: Mutex<u64>,
    closing: AtomicBool,
}
impl ParentCapture {
    pub(crate) fn new(session: &str) -> io::Result<Arc<Self>> {
        let facts = crate::screen::os::session_memory::facts();
        let admission = frame_memory::admit_session(
            facts,
            CapacityCeiling::desktop(facts),
            MAX_WORKING_BYTES,
            session,
        )?;
        admission.bind_contract(None)?;
        Ok(Arc::new(Self {
            admission,
            entries: Mutex::new(Vec::new()),
            last_id: Mutex::new(0),
            closing: AtomicBool::new(false),
        }))
    }
    pub(crate) fn descriptor(&self) -> Admission {
        Admission {
            session: self.admission.session().into(),
            working: self.admission.working_bytes(),
        }
    }
    pub(crate) fn approve(&self, id: u64, descriptor: Plan) -> io::Result<GrantReply> {
        let mut last = self
            .last_id
            .lock()
            .map_err(|_| io::Error::other("login capture sequence poisoned"))?;
        if self.closing.load(Ordering::Acquire) {
            return Err(super::denied("login capture authority is retiring"));
        }
        if id <= *last {
            return Err(super::denied("replayed login capture request"));
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| io::Error::other("login capture owners poisoned"))?;
        let main_exists = entries.iter().any(|entry| {
            entry.descriptor.raw_bytes != 0
                && entry.descriptor.source == descriptor.source
                && entry.descriptor.generation == descriptor.generation
        });
        if (descriptor.raw_bytes != 0 && main_exists) || (descriptor.raw_bytes == 0 && !main_exists)
        {
            return Err(super::denied(
                "login capture main/supplemental generation binding differs",
            ));
        }
        entries
            .try_reserve(1)
            .map_err(|_| io::Error::other("login capture owner allocation failed"))?;
        let grant = self
            .admission
            .authority()
            .approve(descriptor.capture_plan(self.admission.session(), true)?)?;
        *last = id;
        entries.push(Entry {
            id,
            descriptor: descriptor.clone(),
            grant,
        });
        Ok(GrantReply {
            id,
            approved: true,
            plan: descriptor,
        })
    }
    pub(crate) fn close(&self) {
        let _sequence = self
            .last_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.closing.store(true, Ordering::Release);
    }
    #[cfg(test)]
    pub(crate) fn from_test_admission(admission: Arc<SessionAdmission>) -> Arc<Self> {
        Arc::new(Self {
            admission,
            entries: Mutex::new(Vec::new()),
            last_id: Mutex::new(0),
            closing: AtomicBool::new(false),
        })
    }
    pub(crate) fn retire(&self, id: u64) -> io::Result<()> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| io::Error::other("login capture owners poisoned"))?;
        let index = entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| super::denied("unknown retired login capture grant"))?;
        entries.remove(index);
        Ok(())
    }
    pub(crate) fn pixels(&self, reply: &Reply, length: usize) -> io::Result<AdmittedBytes> {
        let id = reply
            .capture_id
            .ok_or_else(|| super::denied("login pixels have no parent grant"))?;
        if reply.state != "pixels"
            || length != reply.pixel_bytes
            || length != crate::login::core::capture_wire::raw_length(reply.width, reply.height)?
        {
            return Err(super::denied(
                "login pixel header differs from capture reply",
            ));
        }
        let entries = self
            .entries
            .lock()
            .map_err(|_| io::Error::other("login capture owners poisoned"))?;
        let entry = entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| super::denied("login pixels refer to an absent grant"))?;
        if entry.descriptor.raw_bytes == 0 || length as u64 > entry.descriptor.raw_bytes {
            return Err(super::denied(
                "login pixels exceed their disjoint parent grant",
            ));
        }
        AdmittedBytes::new(&entry.grant, CaptureOwner::Transport, length as u64)
    }
}
