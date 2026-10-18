//! Platform frame of se Desktop (DSK-M3): the shell `--desktop-shell`, the
//! surfaces' channel and model feed, the transitions of the module with
//! their ledger, the status and the start switches. `shared/` holds the
//! portable parts; `windows/` and `linux_os/` the adapters, chosen here.
use std::ffi::{OsStr, OsString};

#[path = "shared/control.rs"]
pub(crate) mod control;
#[path = "shared/control_types.rs"]
pub(crate) mod control_types;
#[path = "shared/crash_rule.rs"]
pub(crate) mod crash_rule;
// DSK-M4: file actions, sources of the model, the arrangement's only
// writer, opening from the surface and the wallpaper image.
#[path = "shared/desktop_ops.rs"]
pub(crate) mod desktop_ops;
#[path = "shared/feed_sources.rs"]
pub(crate) mod feed_sources;
#[path = "shared/launch.rs"]
pub(crate) mod launch;
#[path = "shared/layout_store.rs"]
pub(crate) mod layout_store;
#[path = "shared/model_feed.rs"]
pub(crate) mod model_feed;
#[path = "shared/open_target.rs"]
pub(crate) mod open_target;
#[path = "shared/status.rs"]
pub(crate) mod status;
#[path = "shared/store.rs"]
pub(crate) mod store;
#[path = "shared/supervisor.rs"]
pub(crate) mod supervisor;
#[path = "shared/supervisor_surface.rs"]
pub(crate) mod supervisor_surface;
#[path = "shared/surface_link.rs"]
pub(crate) mod surface_link;
#[path = "shared/wallpaper_image.rs"]
pub(crate) mod wallpaper_image;

#[cfg(target_os = "linux")]
#[path = "linux_os/mod.rs"]
pub(crate) mod platform;
#[cfg(windows)]
#[path = "windows/mod.rs"]
pub(crate) mod platform;

#[cfg(test)]
#[path = "shared/control_tests.rs"]
mod control_tests;
#[cfg(test)]
#[path = "shared/preserve_tests.rs"]
mod preserve_tests;
#[cfg(test)]
#[path = "shared/process_task_tests.rs"]
mod process_task_tests;

use crate::app::start_request::{
    self, StartRequest, DESKTOP_JUMPLIST_CLEAR, DESKTOP_SHELL, DESKTOP_SURFACE,
};
use crate::desktop::core::config::{DesktopConfig, ForeignDesktopIcons};
use crate::desktop::core::mechanism::{self, Mechanism, SessionFacts};
use crate::desktop::core::state::{Part, PartSet, Platform};

/// One monitor of the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Monitor {
    /// Arrangement key (`layout/<key>.json`), also the surface argument.
    pub(crate) key: String,
    pub(crate) primary: bool,
    /// `[left, top, right, bottom]` in physical pixels, when known.
    pub(crate) rect: Option<[i32; 4]>,
}

/// The parts this session offers and why the others are missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Session {
    pub(crate) facts: SessionFacts,
    pub(crate) mechanism: Mechanism,
    pub(crate) available: PartSet,
    pub(crate) unavailable: Vec<(Part, String)>,
    /// Short codes of `unavailable` (`windows_only`, mechanism reasons).
    pub(crate) unavailable_codes: Vec<(Part, &'static str)>,
    pub(crate) transparent: Result<(), String>,
}

/// Reason of the Explorer parts outside Windows (DSK-M3 contract).
pub(crate) const WINDOWS_ONLY: &str = "nur unter Windows (Explorer)";

impl Session {
    pub(crate) fn probe(config: Option<&DesktopConfig>) -> Session {
        Session::from_facts(platform::PLATFORM, platform::session_facts(), config)
    }

    pub(crate) fn from_facts(
        platform: Platform,
        facts: SessionFacts,
        config: Option<&DesktopConfig>,
    ) -> Session {
        let icons = config.map_or(ForeignDesktopIcons::Keep, |config| {
            config.foreign_desktop_icons
        });
        let mechanism = mechanism::decide(&facts, icons);
        let transparent = mechanism::transparent_allowed(&mechanism, &facts);
        let mut available = PartSet::default();
        let mut unavailable = Vec::new();
        let mut unavailable_codes = Vec::new();
        for part in Part::ALL {
            if !part.exists_on(platform) {
                unavailable.push((part, WINDOWS_ONLY.to_string()));
                unavailable_codes.push((part, "windows_only"));
                continue;
            }
            match (part, &mechanism) {
                (Part::Surface, Mechanism::Unavailable(reason)) => {
                    unavailable.push((part, reason.text()));
                    unavailable_codes.push((part, reason.code()));
                }
                _ => available.set(part, true),
            }
        }
        Session {
            facts,
            mechanism,
            available,
            unavailable,
            unavailable_codes,
            transparent,
        }
    }
}

fn is_desktop_switch(arg: &OsStr) -> bool {
    [DESKTOP_SHELL, DESKTOP_SURFACE, DESKTOP_JUMPLIST_CLEAR]
        .iter()
        .any(|switch| arg == OsStr::new(switch))
}

/// `run_gui` after the `--sync-daemon` branch (V9): the background kinds of
/// se Desktop. `None` when the arguments start none of them; else the exit
/// code of the process.
pub(crate) fn run_if_requested(raw_args: &[OsString]) -> Option<i32> {
    if !raw_args.first().is_some_and(|arg| is_desktop_switch(arg)) {
        return None;
    }
    match start_request::parse(raw_args) {
        Ok(StartRequest::DesktopShell) => Some(supervisor::run_shell()),
        Ok(StartRequest::DesktopSurface { target }) => {
            Some(crate::app::desktop_surface_app::run(&target))
        }
        Ok(StartRequest::DesktopJumplistClear) => Some(match platform::clear_jump_list_here() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("smart_explorer: {error}");
                1
            }
        }),
        Ok(_) => None,
        Err(error) => {
            eprintln!("smart_explorer: {error}");
            Some(2)
        }
    }
}

/// The `desktop` object of `se desktop status --json` and MCP `status`.
pub(crate) fn status_value() -> serde_json::Value {
    status_report().json()
}

pub(crate) fn status_report() -> status::StatusReport {
    let integration = platform::PlatformIntegration::new(launch::gui_binary().ok());
    status::gather(&store::DesktopDir::production(), &integration)
}

/// `--unregister` of the uninstaller (GUI image): every fixed trace and
/// every ledger entry again, including the jump list (C8, Ä25).
pub(crate) fn remove_system_traces() {
    launch::with_control(|control| {
        let outcome = control.remove_system_traces();
        for error in outcome.errors {
            eprintln!("se Desktop: {error}");
        }
    });
}

/// C5.6: in every normal GUI start after the updater steps, also with
/// `--updated`; returns at once and starts the shell in a thread.
pub(crate) fn resume_on_gui_start() {
    if let Ok(gui) = std::env::current_exe() {
        let _ = launch::resume_with(gui, None, launch::start_shell);
    }
}
