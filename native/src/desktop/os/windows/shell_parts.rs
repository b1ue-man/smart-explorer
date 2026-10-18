//! The Explorer parts the shell keeps current (DSK-M5, C1.4, C1.5, C5.5):
//! the navigation-pane root exists exactly while the part `nav_pane` is on
//! and S2 reports the device tree mounted (`TargetFolderPath = root_path`);
//! the jump list follows the device tiles once a changed list stayed the
//! same for one feed tick. Both are written ledger first: under
//! `state.lock`, after reading `state.json` again, the trace goes into the
//! ledger, then into the system. So `se desktop disable|remove|part … off`,
//! which remove under the same lock, never miss a trace, and a part switched
//! off in between is never written again.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::jump_list::{self, JumpPlan};
use super::nav_pane;
use crate::desktop::core::ledger::{
    LedgerEntry, Registration, DESKTOP_NAV_CLSID, JUMP_LIST_TARGET,
};
use crate::desktop::core::state::{ModuleState, Part, State};
use crate::desktop::core::tiles::TileModel;
use crate::desktop::os::feed_sources::DeviceTreeView;
use crate::desktop::os::model_feed::FEED_TICK;
use crate::desktop::os::store::{unix_now, DesktopDir};

/// After a failed write the part waits this long before the next attempt.
const RETRY_AFTER: Duration = Duration::from_secs(60);

pub(crate) struct ShellParts {
    dir: DesktopDir,
    /// The GUI program the links and the icon name.
    gui: PathBuf,
    /// `TargetFolderPath` this shell wrote or found; `None` before the first
    /// round and while no root exists.
    nav_root: Option<String>,
    first_round: bool,
    nav_retry_at: Option<Instant>,
    written: Option<JumpPlan>,
    pending: Option<(JumpPlan, Instant)>,
    jump_retry_at: Option<Instant>,
}

fn recorded(state: &ModuleState, kind: Registration, target: &str) -> bool {
    state
        .ledger
        .iter()
        .any(|entry| entry.kind == kind && entry.target == target)
}

fn on(state: &ModuleState, part: Part) -> bool {
    state.state == State::Enabled && state.parts.contains(part)
}

fn waiting(retry_at: Option<Instant>) -> bool {
    retry_at.is_some_and(|at| Instant::now() < at)
}

impl ShellParts {
    pub(crate) fn new(dir: DesktopDir, gui: PathBuf) -> ShellParts {
        ShellParts {
            dir,
            gui,
            nav_root: None,
            first_round: true,
            nav_retry_at: None,
            written: None,
            pending: None,
            jump_retry_at: None,
        }
    }

    /// One round of the shell: `state` as read this round, the device tree
    /// and the tiles of the feed (`None` before its first projection).
    pub(crate) fn sync(
        &mut self,
        state: &ModuleState,
        tree: &DeviceTreeView,
        tiles: Option<&TileModel>,
    ) {
        if std::mem::take(&mut self.first_round) {
            self.nav_root = nav_pane::current_target();
        }
        self.sync_nav_pane(state, tree);
        self.sync_jump_list(state, tiles);
    }

    fn sync_nav_pane(&mut self, state: &ModuleState, tree: &DeviceTreeView) {
        let wanted = tree
            .root_path()
            .filter(|_| on(state, Part::NavPane))
            .map(str::to_string);
        let present = nav_pane::present();
        match wanted {
            Some(root) => {
                let current = self.nav_root.as_deref() == Some(root.as_str())
                    && present
                    && recorded(state, Registration::NavPaneClsid, DESKTOP_NAV_CLSID);
                if !current && !waiting(self.nav_retry_at) {
                    self.apply_nav_pane(&root);
                }
            }
            None if (present || self.nav_root.is_some()) && !waiting(self.nav_retry_at) => {
                self.remove_nav_pane();
            }
            None => {}
        }
    }

    /// Ledger first, then the twelve values (or only step 8 for a new root).
    fn apply_nav_pane(&mut self, root: &str) {
        let Ok(_lock) = self.dir.lock() else {
            return;
        };
        let Ok(Some(mut state)) = self.dir.read_state() else {
            return;
        };
        if !on(&state, Part::NavPane) {
            return;
        }
        let now = unix_now();
        let parents = nav_pane::missing_parents();
        for target in parents
            .iter()
            .map(String::as_str)
            .chain([DESKTOP_NAV_CLSID])
        {
            state.record(LedgerEntry {
                kind: Registration::NavPaneClsid,
                target: target.to_string(),
                applied_at: now,
            });
        }
        if let Err(error) = self.dir.write_state(&state) {
            self.dir.log("nav_pane_failed", &error);
            self.nav_retry_at = Some(Instant::now() + RETRY_AFTER);
            return;
        }
        let result = if nav_pane::current_target().is_some() {
            nav_pane::retarget(root)
        } else {
            nav_pane::apply(&self.gui, root)
        };
        match result {
            Ok(()) => {
                self.dir.log("nav_pane", root);
                self.nav_root = Some(root.to_string());
                self.nav_retry_at = None;
            }
            // The ledger keeps the entries: removing is idempotent.
            Err(error) => {
                self.dir.log("nav_pane_failed", &error);
                self.nav_retry_at = Some(Instant::now() + RETRY_AFTER);
            }
        }
    }

    /// Created parent keys first (each removes the root before it), then
    /// the root itself; removed traces leave the ledger.
    fn remove_nav_pane(&mut self) {
        let Ok(_lock) = self.dir.lock() else {
            return;
        };
        let mut state = self.dir.read_state().ok().flatten();
        let mut targets: Vec<String> = state
            .iter()
            .flat_map(|state| state.ledger.iter())
            .filter(|entry| entry.kind == Registration::NavPaneClsid)
            .map(|entry| entry.target.clone())
            .filter(|target| target != DESKTOP_NAV_CLSID)
            .collect();
        targets.push(DESKTOP_NAV_CLSID.to_string());
        let mut failed = false;
        for target in targets {
            let result = if target == DESKTOP_NAV_CLSID {
                nav_pane::remove()
            } else {
                nav_pane::remove_parent(&target)
            };
            match result {
                Ok(()) => {
                    if let Some(state) = state.as_mut() {
                        state.forget(Registration::NavPaneClsid, &target);
                    }
                }
                Err(error) => {
                    self.dir.log("nav_pane_failed", &error);
                    failed = true;
                }
            }
        }
        if let Some(state) = &state {
            if let Err(error) = self.dir.write_state(state) {
                self.dir.log("nav_pane_failed", &error);
            }
        }
        if failed {
            self.nav_retry_at = Some(Instant::now() + RETRY_AFTER);
        } else {
            self.dir.log("nav_pane", "entfernt");
            self.nav_retry_at = None;
        }
        self.nav_root = None;
    }

    fn sync_jump_list(&mut self, state: &ModuleState, tiles: Option<&TileModel>) {
        if !on(state, Part::JumpList) || !recorded(state, Registration::JumpList, JUMP_LIST_TARGET)
        {
            // Off (removed by `se desktop`) or never written: write anew.
            self.written = None;
        }
        if !on(state, Part::JumpList) {
            self.pending = None;
            return;
        }
        let Some(tiles) = tiles else {
            return;
        };
        let plan = jump_list::plan_for(tiles);
        if self.written.as_ref() == Some(&plan) {
            self.pending = None;
            return;
        }
        match &self.pending {
            Some((pending, since)) if *pending == plan => {
                if since.elapsed() >= FEED_TICK && !waiting(self.jump_retry_at) {
                    self.write_jump_list(plan);
                }
            }
            _ => self.pending = Some((plan, Instant::now())),
        }
    }

    fn write_jump_list(&mut self, plan: JumpPlan) {
        let Ok(_lock) = self.dir.lock() else {
            return;
        };
        let Ok(Some(mut state)) = self.dir.read_state() else {
            return;
        };
        if !on(&state, Part::JumpList) {
            return;
        }
        if !recorded(&state, Registration::JumpList, JUMP_LIST_TARGET) {
            state.record(LedgerEntry {
                kind: Registration::JumpList,
                target: JUMP_LIST_TARGET.into(),
                applied_at: unix_now(),
            });
            if let Err(error) = self.dir.write_state(&state) {
                self.dir.log("jump_list_failed", &error);
                self.jump_retry_at = Some(Instant::now() + RETRY_AFTER);
                return;
            }
        }
        match jump_list::write(&self.gui, &plan) {
            Ok(shown) => {
                self.dir.log("jump_list", &format!("{shown} Geräte"));
                self.written = Some(plan);
                self.pending = None;
                self.jump_retry_at = None;
            }
            Err(error) => {
                self.dir.log("jump_list_failed", &error);
                self.jump_retry_at = Some(Instant::now() + RETRY_AFTER);
            }
        }
    }
}
