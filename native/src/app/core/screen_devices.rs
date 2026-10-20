//! The devices the desktop viewer can open (SCR-M7, U25-3/10): the Direct
//! contacts with a known device ID and the members of the rooms this device
//! joined, with the online state the share service reports. The profiles
//! come from the background worker on a thread of their own (the call may
//! start the worker), once at the start and then every few seconds, so the
//! GUI thread never waits for them.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use eframe::egui;

use super::screen_target::ScreenTarget;
use crate::share::{device_view, ShareProfiles};

/// Pause between two reads of the profiles.
const REFRESH: Duration = Duration::from_secs(5);

/// Which devices the group view shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GridScope {
    AllDevices,
    /// A room by its local profile ID, room ID or name.
    Room(String),
}

/// One device of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceEntry {
    pub target: ScreenTarget,
    pub name: String,
    /// The room's name, `None` for a Direct device.
    pub room: Option<String>,
    /// Reachable now; others show „Offline“ and are never connected to.
    pub online: bool,
}

/// The Direct devices first, then the members of each joined room.
pub(crate) fn entries(profiles: &ShareProfiles, now: i64) -> Vec<DeviceEntry> {
    let direct = profiles.direct_contacts.iter().filter_map(|contact| {
        let device_id = contact.remote_device_id.as_deref()?;
        Some(DeviceEntry {
            target: ScreenTarget::direct(&contact.id, Some(device_id)),
            name: contact.display_name.clone(),
            room: None,
            online: device_view::online(contact, now),
        })
    });
    let rooms = profiles.rooms.iter().filter(|room| room.auto_join);
    let members = rooms.flat_map(|room| {
        room.members
            .iter()
            .filter(|member| member.is_admitted())
            .map(move |member| DeviceEntry {
                target: ScreenTarget::room(&room.id, &member.device_id),
                name: member.device_name.clone(),
                room: Some(room.name.clone()),
                online: device_view::reachable(&member.status),
            })
    });
    direct.chain(members).collect()
}

impl GridScope {
    /// The devices of `entries` in this scope.
    pub(crate) fn select(
        &self,
        profiles: &ShareProfiles,
        entries: Vec<DeviceEntry>,
    ) -> Vec<DeviceEntry> {
        match self {
            Self::AllDevices => entries,
            Self::Room(wanted) => {
                let room_ids: Vec<&str> = profiles
                    .rooms
                    .iter()
                    .filter(|room| {
                        room.id == *wanted || room.room_id == *wanted || room.name == *wanted
                    })
                    .map(|room| room.id.as_str())
                    .collect();
                entries
                    .into_iter()
                    .filter(|entry| {
                        entry.room.is_some()
                            && room_ids.contains(&entry.target.relation_id.as_str())
                    })
                    .collect()
            }
        }
    }

    /// Whether `profiles` know the room (always true for all devices).
    pub(crate) fn exists(&self, profiles: &ShareProfiles) -> bool {
        match self {
            Self::AllDevices => true,
            Self::Room(wanted) => profiles
                .rooms
                .iter()
                .any(|room| room.id == *wanted || room.room_id == *wanted || room.name == *wanted),
        }
    }
}

/// What the loader thread has read.
#[derive(Default)]
pub(crate) struct DevicesState {
    pub profiles: Option<ShareProfiles>,
    pub error: Option<String>,
    /// Counts the reads, so a user can see a new one.
    pub generation: u64,
}

/// The profiles as the viewer sees them, read in the background.
#[derive(Clone, Default)]
pub(crate) struct Devices {
    state: Arc<Mutex<DevicesState>>,
}

impl Devices {
    /// Starts the loader; it ends when the last handle is dropped.
    pub(crate) fn start(ctx: &egui::Context) -> Self {
        let devices = Self::default();
        let (slot, ctx) = (Arc::downgrade(&devices.state), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("screen-viewer-devices".into())
            .spawn(move || loop {
                let read = crate::daemon::share_worker_snapshot();
                let Some(state) = slot.upgrade() else {
                    return;
                };
                {
                    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.generation += 1;
                    match read {
                        Ok(snapshot) => {
                            state.profiles = Some(snapshot.profiles);
                            state.error = None;
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
                drop(state);
                ctx.request_repaint();
                std::thread::sleep(REFRESH);
            });
        devices
    }

    /// A viewer with fixed profiles (tests).
    #[cfg(test)]
    pub(crate) fn fixed(profiles: ShareProfiles) -> Self {
        let devices = Self::default();
        {
            let mut state = devices.lock();
            state.profiles = Some(profiles);
            state.generation = 1;
        }
        devices
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, DevicesState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
