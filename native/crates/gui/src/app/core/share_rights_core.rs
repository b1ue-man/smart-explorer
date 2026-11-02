//! The real [`RightsBackend`] of the desktop: each button is one profile
//! transaction of the core (`connection_rights_actions.rs`; „Bestätigen“
//! with a change of commands, terminal or screen goes through the worker's
//! Exec-grant journal as one step, W8). Folders picked
//! in the window become Direct exports inside the same transaction, so
//! „Abbrechen“ leaves no stray export behind.
use super::share_rights_model::RightsBackend;
use crate::share::connection_rights::{ConnectionRights, RightsTarget};
use crate::share::connection_rights_actions::{self as actions, RightsChange};
use crate::share::{ShareProfiles, SharedRoot, DIRECT_EXPORT_SCOPE};

pub(in crate::app) struct CoreRights {
    pub(in crate::app) home: Option<String>,
}

impl CoreRights {
    pub(in crate::app) fn new() -> Self {
        Self {
            home: Some(crate::app::dirs_home().to_string_lossy().replace('\\', "/")),
        }
    }
}

/// Adds the picked folders as Direct exports (once per path) and runs
/// `action` in the same profile transaction.
fn with_added_folders(
    home: Option<String>,
    added: &[(String, String)],
    mut action: impl FnMut(&mut ShareProfiles, i64) -> Result<bool, String>,
) -> Result<RightsChange, String> {
    let now = crate::share::core_now_secs();
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(home, |profiles| {
        // The transaction may run again on a newer stored state.
        changed = false;
        for (label, path) in added {
            let known = profiles
                .default_direct_exports
                .roots
                .iter()
                .any(|root| &root.path == path);
            if !known {
                profiles
                    .default_direct_exports
                    .roots
                    .push(SharedRoot::new_in_scope(label, path, DIRECT_EXPORT_SCOPE));
                changed = true;
            }
        }
        changed |= action(profiles, now)?;
        Ok(())
    })?;
    Ok(RightsChange { profiles, changed })
}

impl RightsBackend for CoreRights {
    fn confirm(
        &mut self,
        target: &RightsTarget,
        rights: &ConnectionRights,
        added: &[(String, String)],
    ) -> Result<RightsChange, String> {
        // One step with the picked folders; a change of commands, terminal
        // or screen goes through the worker's Exec-grant journal (W8).
        actions::confirm_connection_rights_with_folders(self.home.clone(), target, rights, added)
    }

    fn cancel(&mut self, target: &RightsTarget) -> Result<RightsChange, String> {
        actions::cancel_connection_rights(self.home.clone(), target)
    }

    fn set_code_rights(
        &mut self,
        rights: Option<&ConnectionRights>,
        added: &[(String, String)],
    ) -> Result<RightsChange, String> {
        if added.is_empty() {
            return actions::set_code_rights(self.home.clone(), rights.cloned());
        }
        if let Some(rights) = rights {
            actions::validate_rights_for_this_host(rights)?;
        }
        with_added_folders(self.home.clone(), added, |profiles, _| {
            profiles.set_code_rights(rights.cloned())
        })
    }
}
