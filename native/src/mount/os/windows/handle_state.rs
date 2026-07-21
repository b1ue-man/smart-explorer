use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};

use crate::mount::{DeleteToken, MountEngine};

mod validation;

use self::validation::{
    check_share_compatibility, matching_delete_type, snapshot_record, validate_record,
};
use super::handle_access::{
    callback_path_key, invalid_handle, require_delete_access, same_or_descendant,
    sharing_violation, FILE_SHARE_DELETE,
};
use super::handle_reservation::{reserved_handle_missing, HandleReservation, RenameReservation};
use super::handle_types::{HandleSnapshot, NodeHandle};

struct HandleRecord {
    node: Option<NodeHandle>,
    path: String,
    is_directory: bool,
    desired_access: u32,
    share_access: u32,
    share_active: bool,
    namespace_attached: bool,
    delete_requested: bool,
    delete_committed: bool,
}

struct PendingDelete {
    token: DeleteToken,
    is_directory: bool,
    requesters: HashSet<u64>,
}

#[derive(Default)]
struct State {
    handles: HashMap<u64, HandleRecord>,
    pending_deletes: HashMap<String, PendingDelete>,
}

pub(super) struct HandleTable {
    state: Mutex<State>,
    namespace_transition: Mutex<()>,
    next_context: AtomicU64,
    case_sensitive_paths: bool,
}

impl HandleTable {
    pub(super) fn new(case_sensitive_paths: bool) -> Self {
        Self {
            state: Mutex::new(State::default()),
            namespace_transition: Mutex::new(()),
            next_context: AtomicU64::new(1),
            case_sensitive_paths,
        }
    }

    pub(super) fn reserve(
        &self,
        path: &str,
        is_directory: bool,
        desired_access: u32,
        share_access: u32,
    ) -> io::Result<HandleReservation<'_>> {
        let transition = self.lock_transition()?;
        let path = self.path_key(path);
        let key = {
            let mut state = self.lock_state()?;
            if state.pending_deletes.contains_key(&path) {
                return Err(sharing_violation("mounted path is pending deletion"));
            }
            check_share_compatibility(&state, &path, desired_access, share_access)?;
            let key = self.allocate_key(&state)?;
            state.handles.insert(
                key,
                HandleRecord {
                    node: None,
                    path,
                    is_directory,
                    desired_access,
                    share_access,
                    share_active: true,
                    namespace_attached: true,
                    delete_requested: false,
                    delete_committed: false,
                },
            );
            key
        };
        Ok(HandleReservation::new(self, key, transition))
    }

    pub(super) fn snapshot(&self, key: u64) -> io::Result<HandleSnapshot> {
        let state = self.lock_state()?;
        snapshot_record(
            state
                .handles
                .get(&key)
                .ok_or_else(|| invalid_handle("unknown file handle"))?,
        )
    }

    pub(super) fn cleanup(&self, key: u64) -> io::Result<HandleSnapshot> {
        let mut state = self.lock_state()?;
        let record = state
            .handles
            .get_mut(&key)
            .ok_or_else(|| invalid_handle("unknown file handle"))?;
        record.share_active = false;
        snapshot_record(record)
    }

    pub(super) fn take(&self, key: u64) -> io::Result<HandleSnapshot> {
        let mut state = self.lock_state()?;
        let record = state
            .handles
            .remove(&key)
            .ok_or_else(|| invalid_handle("unknown file handle"))?;
        if record.delete_requested {
            if let Some(delete) = state.pending_deletes.get_mut(&record.path) {
                delete.requesters.remove(&key);
            }
        }
        snapshot_record(&record)
    }

    pub(super) fn request_delete(
        &self,
        engine: &MountEngine,
        key: u64,
        path: &str,
        is_directory: bool,
    ) -> io::Result<()> {
        let _transition = self.lock_transition()?;
        self.request_delete_locked(engine, key, path, is_directory)
    }

    pub(super) fn cancel_delete(
        &self,
        engine: &MountEngine,
        key: u64,
        path: &str,
        is_directory: bool,
    ) -> io::Result<()> {
        let _transition = self.lock_transition()?;
        let path = self.path_key(path);
        let token = {
            let mut state = self.lock_state()?;
            validate_record(&state, key, &path, is_directory)?;
            let Some(delete) = state.pending_deletes.get_mut(&path) else {
                return Ok(());
            };
            matching_delete_type(delete, is_directory)?;
            if !delete.requesters.contains(&key) {
                return Ok(());
            }
            if delete.requesters.len() > 1 {
                delete.requesters.remove(&key);
                if let Some(record) = state.handles.get_mut(&key) {
                    record.delete_requested = false;
                }
                return Ok(());
            }
            delete.token
        };
        engine.cancel_delete(token)?;
        let mut state = self.lock_state()?;
        state.pending_deletes.remove(&path);
        if let Some(record) = state.handles.get_mut(&key) {
            record.delete_requested = false;
        }
        Ok(())
    }

    pub(super) fn commit_delete(
        &self,
        engine: &MountEngine,
        key: u64,
        path: &str,
        is_directory: bool,
    ) -> io::Result<()> {
        let _transition = self.lock_transition()?;
        let path = self.path_key(path);
        let token = {
            let state = self.lock_state()?;
            let record = validate_record(&state, key, &path, is_directory)?;
            if record.delete_committed {
                return Ok(());
            }
            let delete = state.pending_deletes.get(&path).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "delete-pending object has no prepared delete transaction",
                )
            })?;
            matching_delete_type(delete, is_directory)?;
            if !delete.requesters.contains(&key) {
                return Err(invalid_handle("file handle did not request deletion"));
            }
            delete.token
        };
        engine.commit_delete(token)?;
        let mut state = self.lock_state()?;
        let requesters = state
            .pending_deletes
            .remove(&path)
            .map(|delete| delete.requesters)
            .unwrap_or_default();
        for (handle_key, record) in &mut state.handles {
            if record.namespace_attached && record.path == path {
                record.namespace_attached = false;
            }
            if requesters.contains(handle_key) {
                record.delete_requested = false;
                record.delete_committed = true;
            }
        }
        Ok(())
    }

    pub(super) fn reserve_rename(
        &self,
        key: u64,
        source: &str,
        destination: &str,
        replace_existing: bool,
    ) -> io::Result<RenameReservation<'_>> {
        let transition = self.lock_transition()?;
        let source = self.path_key(source);
        let destination = self.path_key(destination);
        let destination_is_open = {
            let state = self.lock_state()?;
            let caller = state
                .handles
                .get(&key)
                .ok_or_else(|| invalid_handle("rename has no open source handle"))?;
            if !caller.share_active || !caller.namespace_attached || caller.path != source {
                return Err(invalid_handle(
                    "rename source does not match its open handle",
                ));
            }
            require_delete_access(caller.desired_access)?;
            if state.pending_deletes.keys().any(|path| {
                same_or_descendant(path, &source)
                    || same_or_descendant(path, &destination)
                    || same_or_descendant(&destination, path)
            }) {
                return Err(sharing_violation(
                    "delete-pending namespace prevents rename",
                ));
            }
            for (other_key, record) in &state.handles {
                if *other_key != key
                    && record.share_active
                    && record.namespace_attached
                    && record.path == source
                    && record.share_access & FILE_SHARE_DELETE == 0
                {
                    return Err(sharing_violation(
                        "an open source handle does not share delete access",
                    ));
                }
            }
            let mut found = false;
            if replace_existing {
                for record in state
                    .handles
                    .values()
                    .filter(|record| record.namespace_attached && record.path == destination)
                {
                    found = true;
                    if record.share_active && record.share_access & FILE_SHARE_DELETE == 0 {
                        return Err(sharing_violation(
                            "an open destination handle does not share delete access",
                        ));
                    }
                }
            }
            found
        };
        Ok(RenameReservation::new(
            self,
            transition,
            source,
            destination,
            replace_existing,
            destination_is_open,
        ))
    }

    pub(super) fn request_delete_locked(
        &self,
        engine: &MountEngine,
        key: u64,
        path: &str,
        is_directory: bool,
    ) -> io::Result<()> {
        let path = self.path_key(path);
        {
            let mut state = self.lock_state()?;
            let record = validate_record(&state, key, &path, is_directory)?;
            if record.delete_committed || record.delete_requested {
                return Ok(());
            }
            require_delete_access(record.desired_access)?;
            for (other_key, other) in &state.handles {
                if *other_key != key
                    && other.share_active
                    && other.namespace_attached
                    && other.path == path
                    && other.share_access & FILE_SHARE_DELETE == 0
                {
                    return Err(sharing_violation(
                        "an open handle does not share delete access",
                    ));
                }
            }
            if let Some(delete) = state.pending_deletes.get_mut(&path) {
                matching_delete_type(delete, is_directory)?;
                delete.requesters.insert(key);
                let requester = state
                    .handles
                    .get_mut(&key)
                    .ok_or_else(|| invalid_handle("delete requester disappeared"))?;
                requester.delete_requested = true;
                return Ok(());
            }
        }
        let token = engine.begin_delete(&path, is_directory)?;
        let mut state = self.lock_state()?;
        validate_record(&state, key, &path, is_directory)?;
        state.pending_deletes.insert(
            path,
            PendingDelete {
                token,
                is_directory,
                requesters: HashSet::from([key]),
            },
        );
        state
            .handles
            .get_mut(&key)
            .ok_or_else(|| invalid_handle("delete requester disappeared"))?
            .delete_requested = true;
        Ok(())
    }

    fn allocate_key(&self, state: &State) -> io::Result<u64> {
        for _ in 0..u16::MAX {
            let candidate = self.next_context.fetch_add(1, Ordering::Relaxed);
            if candidate != 0 && !state.handles.contains_key(&candidate) {
                return Ok(candidate);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            "no callback context identifier is available",
        ))
    }

    fn path_key(&self, path: &str) -> String {
        callback_path_key(path, self.case_sensitive_paths)
    }

    fn lock_state(&self) -> io::Result<MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("file-handle state is unavailable"))
    }

    fn lock_transition(&self) -> io::Result<MutexGuard<'_, ()>> {
        self.namespace_transition
            .lock()
            .map_err(|_| io::Error::other("file-handle namespace state is unavailable"))
    }
    pub(super) fn bind_reserved(&self, key: u64, node: NodeHandle) -> io::Result<()> {
        let mut state = self.lock_state()?;
        let record = state
            .handles
            .get_mut(&key)
            .ok_or_else(reserved_handle_missing)?;
        if record.node.is_some() {
            return Err(invalid_handle("reserved file handle is already bound"));
        }
        if record.is_directory != matches!(node, NodeHandle::Directory) {
            return Err(invalid_handle("reserved file handle type changed"));
        }
        record.node = Some(node);
        Ok(())
    }

    pub(super) fn reserved_path(&self, key: u64) -> io::Result<String> {
        self.lock_state()?
            .handles
            .get(&key)
            .map(|record| record.path.clone())
            .ok_or_else(reserved_handle_missing)
    }

    pub(super) fn abort_reservation(&self, key: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.handles.remove(&key);
        }
    }

    pub(super) fn complete_rename(
        &self,
        source: &str,
        destination: &str,
        replace_existing: bool,
    ) -> io::Result<()> {
        let mut state = self.lock_state()?;
        if replace_existing {
            for record in state
                .handles
                .values_mut()
                .filter(|record| record.namespace_attached && record.path == destination)
            {
                record.namespace_attached = false;
            }
        }
        let descendant_prefix = format!("{}\\", source.trim_end_matches('\\'));
        for record in state.handles.values_mut() {
            if !record.namespace_attached {
                continue;
            }
            if record.path == source {
                record.path = destination.to_string();
            } else if let Some(suffix) = record.path.strip_prefix(&descendant_prefix) {
                record.path = format!("{}\\{suffix}", destination.trim_end_matches('\\'));
            }
        }
        Ok(())
    }
}
