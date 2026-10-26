//! Prepared imports are registered separately from bound image plans.
use super::*;
use crate::screen::core::{
    capture_import::CaptureImportNeed,
    worker_capture_wire::{GrantRequest, GrantValue},
};
impl CaptureBroker {
    fn approve_supplement(&self, image: u64, plan: CapturePlan) -> io::Result<CreditWire> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("capture broker poisoned"))?;
        if state.exited {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "worker exited"));
        }
        if state.current != Some(image) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "cursor image is no longer current",
            ));
        }
        let image = state
            .entries
            .get(&image)
            .and_then(|entry| entry.worker.as_ref())
            .ok_or_else(|| io::Error::other("cursor image has no worker credit"))?;
        image
            .for_member(plan.source(), plan.generation())?
            .check_supplement(&plan)?;
        let grant = self.admission.authority().approve(plan)?;
        let id = state.next;
        state.next = id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("worker credit IDs exhausted"))?;
        let credit = CreditWire {
            id,
            source: grant.source().source().into(),
            generation: grant.generation().value(),
            total: grant.plan().peak_bytes(),
            owners: grant.plan().owner_limits(),
        };
        state
            .entries
            .try_reserve(1)
            .map_err(|_| io::Error::other("worker credit registry allocation failed"))?;
        state.entries.insert(
            id,
            Entry {
                worker: Some(grant),
                parent: None,
                held: None,
            },
        );
        Ok(credit)
    }
    fn registration_key(
        &self,
        source: &str,
        parent: Option<String>,
        selection: Option<crate::screen::core::window_list::SourceId>,
    ) -> io::Result<SourceKey> {
        let selected = crate::screen::core::window_list::SourceId::parse(self.source.source())?;
        if let Some(parent) = parent {
            let bound = self
                .state
                .lock()
                .map_err(|_| io::Error::other("capture broker poisoned"))?
                .prepared_root
                .clone();
            if selection.is_some()
                || bound.as_ref().map(SourceKey::source) != Some(parent.as_str())
                || crate::screen::core::family_protocol::selector(&selected).is_none()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "foreign family preparation",
                ));
            }
        } else {
            if selection.as_ref() != Some(&selected) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "prepared root has another selected source",
                ));
            }
            self.bind_source(SourceKey::new(self.session(), source)?)?;
        }
        SourceKey::new(self.session(), source)
    }
    fn register_source(
        &self,
        source: String,
        parent: Option<String>,
        selection: Option<crate::screen::core::window_list::SourceId>,
        owned: bool,
    ) -> io::Result<GrantValue> {
        let key = self.registration_key(&source, parent, selection)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("capture broker poisoned"))?;
        if state.exited {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "worker exited"));
        }
        state
            .prepared
            .try_reserve(1)
            .map_err(|_| io::Error::other("prepared registry allocation failed"))?;
        let entry = state
            .prepared
            .entry(source.clone())
            .or_insert_with(|| PreparedOwners {
                manual: false,
                owners: Vec::new(),
            });
        let authority = self.admission.authority();
        if owned {
            entry
                .owners
                .try_reserve(1)
                .map_err(|_| io::Error::other("prepared owner registry allocation failed"))?;
            let owner = authority.own_prepared_source(key)?;
            entry.owners.push(owner);
            Ok(GrantValue::PreparedOwner {
                source,
                owner: owner.value(),
            })
        } else {
            authority.register_prepared_source(key)?;
            entry.manual = true;
            Ok(GrantValue::Registered)
        }
    }
    fn end_prepared(&self, source: String, owner: Option<u64>) {
        let Ok(key) = SourceKey::new(self.session(), &source) else {
            return;
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = state.prepared.get_mut(&source) else {
            return;
        };
        let authority = self.admission.authority();
        if let Some(owner) = owner {
            let Ok(owner) = PreparedSourceOwnerId::new(owner) else {
                return;
            };
            if !entry.owners.contains(&owner) {
                return;
            }
            entry.owners.retain(|id| *id != owner);
            authority.release_prepared_source(&key, owner);
        } else {
            entry.manual = false;
            authority.unregister_prepared_source(&key);
        }
        if !entry.manual && entry.owners.is_empty() {
            state.prepared.remove(&source);
        }
    }
    fn prepared_key(&self, source: &str) -> io::Result<SourceKey> {
        if !self
            .state
            .lock()
            .map_err(|_| io::Error::other("capture broker poisoned"))?
            .prepared
            .contains_key(source)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source has no verified preparation registration",
            ));
        }
        SourceKey::new(self.session(), source)
    }
    fn approve_import(&self, need: CaptureImportNeed) -> io::Result<CreditWire> {
        self.prepared_key(need.source().source())?;
        let grant = self
            .admission
            .authority()
            .approve_import(need)?
            .control_grant();
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("capture broker poisoned"))?;
        if state.exited {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "worker exited during import admission",
            ));
        }
        let id = state.next;
        state.next = id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("worker credit IDs exhausted"))?;
        let credit = CreditWire {
            id,
            source: grant.source().source().into(),
            generation: grant.generation().value(),
            total: grant.plan().peak_bytes(),
            owners: grant.plan().owner_limits(),
        };
        state
            .entries
            .try_reserve(1)
            .map_err(|_| io::Error::other("worker credit registry allocation failed"))?;
        state.entries.insert(
            id,
            Entry {
                worker: Some(grant),
                parent: None,
                held: None,
            },
        );
        Ok(credit)
    }
    pub(in crate::screen::os::host_worker_client) fn resource(
        &self,
        request: GrantRequest,
    ) -> Option<(u64, io::Result<GrantValue>)> {
        match request {
            GrantRequest::FramePlan { request, plan } => Some((
                request,
                plan.plan(self.session())
                    .and_then(|plan| self.approve(plan, None))
                    .map(GrantValue::Frame),
            )),
            GrantRequest::SupplementPlan {
                request,
                image,
                plan,
            } => Some((
                request,
                plan.plan(self.session())
                    .and_then(|plan| self.approve_supplement(image, plan))
                    .map(GrantValue::Supplement),
            )),
            GrantRequest::PreparedImport { request, need } => Some((
                request,
                need.need(self.session())
                    .and_then(|need| self.approve_import(need))
                    .map(GrantValue::Import),
            )),
            GrantRequest::RegisterPrepared {
                request,
                source,
                parent,
                selection,
            } => Some((
                request,
                self.register_source(source, parent, selection, false),
            )),
            GrantRequest::OwnPrepared {
                request,
                source,
                parent,
                selection,
            } => Some((
                request,
                self.register_source(source, parent, selection, true),
            )),
            GrantRequest::BeginImports { request, source } => Some((
                request,
                self.prepared_key(&source)
                    .and_then(|source| self.admission.authority().begin_import_connection(&source))
                    .map(GrantValue::Connection),
            )),
            GrantRequest::EndPrepared { source } => {
                self.end_prepared(source, None);
                None
            }
            GrantRequest::EndPreparedOwner { source, owner } => {
                self.end_prepared(source, Some(owner));
                None
            }
            GrantRequest::EndImports { source, connection } => {
                if let Ok(source) = self.prepared_key(&source) {
                    self.admission
                        .authority()
                        .end_import_connection(&source, connection);
                }
                None
            }
        }
    }
}
