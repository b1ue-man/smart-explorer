//! Keep the connection and backend path together across folder selection.
use super::prelude::*;
use super::*;

pub(in crate::app) struct PickerLocation {
    pub label: String,
    pub root: String,
    pub prefix: String,
    pub remote: bool,
    pub backend: crate::vfs::BackendHandle,
}

impl App {
    pub(in crate::app) fn pane_endpoint(&self, index: usize) -> Result<String, String> {
        let (root, remote) = if index == self.active_tab {
            (&self.root_path, self.remote.as_ref())
        } else {
            let tab = &self.tabs[index];
            (&tab.root_path, tab.remote.as_ref())
        };
        match remote {
            Some(remote) => remote.endpoint_prefix.as_ref()
                .map(|prefix| format!("{prefix}{root}"))
                .ok_or_else(|| "Diese Sitzung hat keine dauerhaft wiederherstellbare Adresse. Sie kann als Quelle direkt gespiegelt werden.".into()),
            None => Ok(crate::connect::local_root(root)),
        }
    }

    pub(in crate::app) fn begin_pane_sync_setup(&mut self, source: usize, target: Option<usize>) {
        let endpoints = self.pane_endpoint(source).and_then(|source| {
            let target = target.map(|index| self.pane_endpoint(index)).transpose()?;
            Ok((source, target.unwrap_or_default()))
        });
        match endpoints {
            Ok((source, target)) => {
                self.job_editor = Some(JobEditor::blank(source, target));
                self.show_sync_jobs = true;
            }
            Err(error) => self.error_msg = Some(error),
        }
    }

    pub(in crate::app) fn picker_tab_locations(&self) -> Vec<PickerLocation> {
        let persistent = self.picker.as_ref().is_some_and(|picker| matches!(
            picker.purpose, PickerPurpose::SyncSource | PickerPurpose::SyncTarget));
        (0..self.tabs.len()).filter_map(|index| {
            let (root, remote) = if index == self.active_tab {
                (&self.root_path, self.remote.as_ref())
            } else {
                (&self.tabs[index].root_path, self.tabs[index].remote.as_ref())
            };
            if root.is_empty() || (persistent && remote.is_some_and(|r| r.endpoint_prefix.is_none())) {
                return None;
            }
            let (backend, root) = self.pane_backend(index);
            let label = remote.map_or_else(|| root.clone(), |r| format!("{}: {root}", r.label));
            Some(PickerLocation {
                label, root, backend, remote: remote.is_some(),
                prefix: remote.and_then(|r| r.endpoint_prefix.clone()).unwrap_or_default(),
            })
        }).collect()
    }

    pub(in crate::app) fn picker_use_location(&mut self, location: PickerLocation) {
        if let Some(picker) = self.picker.as_mut() {
            picker.connect_rx = None;
            picker.connecting = false;
            picker.list_rx = None;
            picker.backend = Some(location.backend);
            picker.cwd = location.root;
            picker.endpoint_prefix = location.prefix;
            picker.is_remote = location.remote;
            picker.conn_label = location.label;
        }
        self.picker_list();
    }

    pub(in crate::app) fn picker_peer_locations(&self) -> Vec<(String, crate::share::PeerOpenTarget)> {
        let mut places: Vec<_> = self.share_profiles.direct_contacts.iter().map(|contact| {
            (contact.display_name.clone(), crate::share::PeerOpenTarget::Direct { contact_id: contact.id.clone() })
        }).collect();
        for room in &self.share_profiles.rooms {
            for member in &room.members {
                places.push((format!("{} / {}", room.name, member.device_name),
                    crate::share::PeerOpenTarget::RoomDevice {
                        room_id: room.id.clone(), device_id: member.device_id.clone(),
                    }));
            }
        }
        places
    }
}
