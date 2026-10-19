//! The surface's view model (DSK-M4, C5.5): the shell's channel fills it
//! (`model`, `config`); the view reads it and sends arrangement changes
//! (`layout {op}`), "Verbinden", "Aktualisieren" and the heartbeat back. A
//! change is shown at once in the own arrangement, so a tile does not jump
//! back until the shell's next model; a new configuration applies at once to
//! the groups shown (`se desktop config` acts immediately).
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::items::{DesktopFile, SystemItem};
use crate::desktop::core::layout::{Cell, DesktopLayout, ItemId, LayoutOp};
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage, SurfaceModel};
use crate::desktop::core::tiles::{DeviceTile, EmptyAction, RoomGroup, ShareState, VaultTile};
use crate::desktop::os::feed_sources::DesktopFolder;
use crate::desktop::os::model_feed::NO_SERVICE;
use crate::desktop::os::surface_link::FrameSender;

/// Heartbeat and frame cadence (C5.5).
pub(crate) const ALIVE_EVERY: Duration = Duration::from_secs(1);

/// Where the surface's messages go: its channel, a recorder in the tests.
pub(crate) trait Outbox {
    fn send(&self, message: SurfaceMessage) -> bool;
}

impl Outbox for FrameSender<SurfaceMessage> {
    fn send(&self, message: SurfaceMessage) -> bool {
        FrameSender::send(self, message)
    }
}

/// One item of the surface with its data.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ItemRef<'a> {
    Device(&'a DeviceTile),
    Room(&'a RoomGroup),
    Vault(&'a VaultTile),
    File(&'a DesktopFile),
    System(SystemItem),
    AllScreens,
    Empty(EmptyAction),
}

pub(crate) struct ViewModel {
    monitor: String,
    model: Option<SurfaceModel>,
    config: DesktopConfig,
    folders: Vec<DesktopFolder>,
    outbox: Box<dyn Outbox>,
    send_alive: bool,
    last_alive: Option<Instant>,
}

impl ViewModel {
    /// `config` until the shell sends its own; `folders` are the desktop
    /// folders whose names the model carries.
    pub(crate) fn new(
        monitor: &str,
        outbox: Box<dyn Outbox>,
        send_alive: bool,
        config: DesktopConfig,
        folders: Vec<DesktopFolder>,
    ) -> ViewModel {
        ViewModel {
            monitor: monitor.to_string(),
            model: None,
            config,
            folders,
            outbox,
            send_alive,
            last_alive: None,
        }
    }

    /// Takes every waiting message of the shell; `true` when one changed
    /// what the surface shows.
    pub(crate) fn drain(&mut self, inbox: &Receiver<ShellMessage>) -> bool {
        let mut changed = false;
        while let Ok(message) = inbox.try_recv() {
            changed |= self.absorb(message);
        }
        changed
    }

    pub(crate) fn absorb(&mut self, message: ShellMessage) -> bool {
        match message {
            ShellMessage::Model { model, .. } => {
                self.model = Some(*model);
                true
            }
            ShellMessage::Config { config } => {
                let changed = config != self.config;
                self.config = config;
                changed
            }
            // The shell checks the greeting; `stop` ends the process in the
            // channel's reader thread.
            ShellMessage::Hello { .. } | ShellMessage::Stop { .. } => false,
        }
    }

    pub(crate) fn monitor(&self) -> &str {
        &self.monitor
    }

    pub(crate) fn model(&self) -> Option<&SurfaceModel> {
        self.model.as_ref()
    }

    pub(crate) fn config(&self) -> &DesktopConfig {
        &self.config
    }

    /// Rows of the own arrangement (the shell's `Grid` value).
    pub(crate) fn rows(&self) -> Option<u32> {
        self.model.as_ref().map(|model| model.layout.rows)
    }

    /// Sends an arrangement change and shows it at once.
    pub(crate) fn layout_op(&mut self, op: LayoutOp) {
        if let Some(model) = self.model.as_mut() {
            let mut desktop = DesktopLayout::default();
            desktop
                .monitors
                .insert(self.monitor.clone(), model.layout.clone());
            if desktop.apply(&op).is_ok() {
                if let Some(layout) = desktop.monitors.remove(&self.monitor) {
                    model.layout = layout;
                }
            }
        }
        self.outbox
            .send(SurfaceMessage::Layout { op, monitor: None });
    }

    /// "Verbinden" (C10): the shell starts the service.
    pub(crate) fn connect(&self) {
        self.outbox.send(SurfaceMessage::Connect);
    }

    /// "Aktualisieren" (F5): the shell reads the wallpaper and sends anew.
    pub(crate) fn refresh(&self) {
        self.outbox.send(SurfaceMessage::WallpaperChanged);
    }

    /// At most once a second after a drawn frame (C5.5).
    pub(crate) fn heartbeat(&mut self) {
        let due = self
            .last_alive
            .is_none_or(|sent| sent.elapsed() >= ALIVE_EVERY);
        if self.send_alive && due && self.outbox.send(SurfaceMessage::Alive) {
            self.last_alive = Some(Instant::now());
        }
    }

    /// The items to draw in reading order: placed in this monitor's
    /// arrangement, present in the model and in a shown group.
    pub(crate) fn visible(&self) -> Vec<(ItemId, Cell)> {
        let Some(model) = &self.model else {
            return Vec::new();
        };
        let mut placed: Vec<(ItemId, Cell)> = model
            .layout
            .cells
            .iter()
            .filter(|(item, _)| self.shown(item))
            .map(|(item, cell)| (item.clone(), *cell))
            .collect();
        placed.sort_by_key(|(_, cell)| (cell.col, cell.row));
        placed
    }

    fn shown(&self, item: &ItemId) -> bool {
        let Some(found) = self.lookup(item) else {
            return false;
        };
        let groups = self.config.groups;
        match found {
            ItemRef::Device(_) => groups.devices,
            ItemRef::Room(_) => groups.rooms,
            ItemRef::Vault(_) => groups.vaults,
            ItemRef::File(_) => groups.desktop_files,
            ItemRef::System(SystemItem::RecycleBin) => self.config.system_items.recycle_bin,
            ItemRef::System(SystemItem::ThisPc) => self.config.system_items.this_pc,
            ItemRef::AllScreens => groups.screens,
            ItemRef::Empty(_) => groups.devices || groups.rooms,
        }
    }

    /// The data of an item; `None` for one the model no longer has.
    pub(crate) fn lookup(&self, item: &ItemId) -> Option<ItemRef<'_>> {
        let model = self.model.as_ref()?;
        let tiles = &model.tiles;
        match item {
            ItemId::Device { key } => tiles
                .devices
                .iter()
                .chain(tiles.rooms.iter().flat_map(|room| room.members.iter()))
                .find(|tile| tile.key == *key)
                .map(ItemRef::Device),
            ItemId::Room { room_id } => tiles
                .rooms
                .iter()
                .find(|room| room.room_id == *room_id)
                .map(ItemRef::Room),
            ItemId::Vault { site } => tiles
                .vaults
                .as_ref()?
                .iter()
                .find(|vault| vault.site == *site)
                .map(ItemRef::Vault),
            ItemId::File { name } => file_of(model, name, false).map(ItemRef::File),
            ItemId::PublicFile { name } => file_of(model, name, true).map(ItemRef::File),
            ItemId::System(system) => model
                .system_items
                .contains(system)
                .then_some(ItemRef::System(*system)),
            ItemId::AllScreens => model.screens.then_some(ItemRef::AllScreens),
            ItemId::AddDevice => empty(tiles.empty_actions.as_slice(), EmptyAction::AddDevice),
            ItemId::JoinRoom => empty(tiles.empty_actions.as_slice(), EmptyAction::JoinRoom),
        }
    }

    /// The folder of a desktop file (the user's or the all-users desktop).
    pub(crate) fn folder(&self, public: bool) -> Option<&Path> {
        self.folders
            .iter()
            .find(|folder| folder.public == public)
            .map(|folder| folder.path.as_path())
    }

    /// Path of a desktop file of the model.
    pub(crate) fn file_path(&self, file: &DesktopFile) -> Option<PathBuf> {
        self.folder(file.public)
            .map(|folder| folder.join(&file.name))
    }

    /// The status lines of the surface (C4.4, C4.6): connection, profiles
    /// that cannot be read, vaults without data.
    pub(crate) fn status_lines(&self) -> Vec<String> {
        let Some(model) = &self.model else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        if let ShareState::Profiles { reason } = &model.tiles.share_state {
            lines.push(format!("Nicht verbunden: {reason}"));
        }
        if let Some(error) = &model.tiles.profiles_error {
            lines.push(format!("Geräte und Räume nicht lesbar: {error}"));
        }
        if let Some(line) = self.vault_line() {
            lines.push(line);
        }
        lines
    }

    /// "Tresore: …" while the group has no data: without the service, or a
    /// service that does not answer `Vault::Status` (state unknown).
    pub(crate) fn vault_line(&self) -> Option<String> {
        let model = self.model.as_ref()?;
        if model.tiles.vaults.is_some() || !self.config.groups.vaults {
            return None;
        }
        Some(match model.tiles.share_state {
            ShareState::Live => "Tresore: Zustand unbekannt".to_string(),
            ShareState::Profiles { .. } => format!("Tresore: {NO_SERVICE}"),
        })
    }

    /// Whether the button "Verbinden" is offered (C4.6, C10).
    pub(crate) fn offers_connect(&self) -> bool {
        self.model
            .as_ref()
            .is_some_and(|model| matches!(model.tiles.share_state, ShareState::Profiles { .. }))
    }
}

fn file_of<'a>(model: &'a SurfaceModel, name: &str, public: bool) -> Option<&'a DesktopFile> {
    model
        .files
        .iter()
        .find(|file| file.public == public && !file.hidden && file.name == name)
}

fn empty(actions: &[EmptyAction], action: EmptyAction) -> Option<ItemRef<'static>> {
    actions.contains(&action).then_some(ItemRef::Empty(action))
}

#[cfg(test)]
#[path = "desktop_surface_task_tests.rs"]
mod task_tests;
#[cfg(test)]
#[path = "desktop_surface_ui_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "desktop_surface_window_tests.rs"]
mod window_tests;
