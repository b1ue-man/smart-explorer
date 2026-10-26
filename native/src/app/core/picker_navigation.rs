//! Picker navigation retains the backend, connection prefix and literal path.
use crate::app::prelude::*;
use crate::app::*;

impl App {
    /// Open the picker to fill a sync-setup field, starting from `initial`
    /// (local path → browse there; remote URL or empty → start at the roots).
    pub(in crate::app) fn open_picker(&mut self, purpose: PickerPurpose, initial: &str) {
        let mut st = PickerState {
            purpose,
            backend: None,
            is_remote: false,
            endpoint_prefix: String::new(),
            conn_label: String::new(),
            cwd: String::new(),
            entries: Vec::new(),
            selected: None,
            list_rx: None,
            listing: false,
            error: None,
            connect_rx: None,
            connecting: false,
        };
        // A local starting folder opens directly; remote/empty starts at roots.
        if !initial.trim().is_empty()
            && !crate::connect::is_remote_url(initial)
            && !initial.contains("://")
        {
            st.backend = Some(Arc::new(crate::vfs::LocalBackend::new("/")));
            st.cwd = crate::connect::local_root(initial);
            if st.cwd.is_empty() {
                st.cwd = "/".into();
            }
        }
        self.picker = Some(st);
        if self
            .picker
            .as_ref()
            .map(|s| s.backend.is_some())
            .unwrap_or(false)
        {
            self.picker_list();
        }
    }

    /// Open a local drive / folder root in the picker.
    pub(in crate::app) fn picker_open_local(&mut self, root: &str) {
        if let Some(p) = self.picker.as_mut() {
            p.backend = Some(Arc::new(crate::vfs::LocalBackend::new("/")));
            p.is_remote = false;
            p.endpoint_prefix = String::new();
            p.conn_label = String::new();
            let c = crate::connect::local_root(root);
            let c = c.trim_end_matches('/');
            p.cwd = if c.is_empty() {
                "/".into()
            } else {
                crate::connect::local_root(c)
            };
            p.connecting = false;
            p.connect_rx = None;
        }
        self.accept_drive_consumer(crate::app::drive_snapshot::DriveConsumer::Picker, root);
        self.picker_list();
    }

    /// Parent of a picker directory (None at a drive/remote root).
    pub(in crate::app) fn picker_parent(p: &str) -> Option<String> {
        let t = p.trim_end_matches('/');
        if t.is_empty() || t == "/" {
            return None;
        }
        if t.len() == 2 && t.ends_with(':') {
            return None; // drive root "C:"
        }
        match t.rsplit_once('/') {
            Some((par, _)) => {
                if par.is_empty() {
                    Some("/".into())
                } else if par.len() == 2 && par.ends_with(':') {
                    Some(format!("{}/", par))
                } else {
                    Some(par.to_string())
                }
            }
            None => None,
        }
    }

    /// The value the picker would return for the current folder.
    pub(in crate::app) fn picker_value(p: &PickerState) -> String {
        if p.is_remote {
            format!("{}{}", p.endpoint_prefix, p.cwd)
        } else {
            p.cwd.clone()
        }
    }
}

pub(super) fn picker_child_path(cwd: &str, name: &str) -> String {
    let trimmed = cwd.trim_end_matches('/');
    if trimmed.is_empty() {
        format!("/{}", name)
    } else {
        format!("{}/{}", trimmed, name)
    }
}
