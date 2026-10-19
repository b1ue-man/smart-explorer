//! The windows of a KDE Plasma session through `org_kde_plasma_window_
//! management` (SCR-M5, U19-7, assumption A6). KWin hands this interface
//! only to programs whose desktop file asks for it with
//! `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` and whose
//! `Exec` line has the absolute path, which is why only the desktop program
//! (not `se`) asks for the list. Each call opens its own connection, binds,
//! runs two round trips and closes it. Windows that skip the task bar
//! (panels, the desktop) stay out of the list.
use std::collections::HashMap;
use std::io;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_window::{
    self, OrgKdePlasmaWindow,
};
use wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_window_management::{
    self, OrgKdePlasmaWindowManagement,
};

use super::wayland_toplevels::clean_id;
use crate::screen::core::window_list::{ListSource, WindowEntry, WindowList};

/// `org_kde_plasma_window_management.state` bits.
const STATE_ACTIVE: u32 = 0x1;
const STATE_MINIMIZED: u32 = 0x2;
const STATE_SKIPTASKBAR: u32 = 0x1000;

#[derive(Default, Clone)]
struct Window {
    title: String,
    app_id: String,
    pid: Option<u32>,
    flags: u32,
}

#[derive(Default)]
struct State {
    windows: HashMap<u32, (String, Window)>,
    order: Vec<u32>,
}

impl State {
    fn window(&mut self, id: u32, key: &str) -> &mut Window {
        if !self.windows.contains_key(&id) {
            self.order.push(id);
        }
        &mut self
            .windows
            .entry(id)
            .or_insert_with(|| (key.to_string(), Window::default()))
            .1
    }

    fn remove(&mut self, id: u32) {
        self.windows.remove(&id);
        self.order.retain(|known| *known != id);
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<OrgKdePlasmaWindowManagement, ()> for State {
    fn event(
        _: &mut Self,
        manager: &OrgKdePlasmaWindowManagement,
        event: org_kde_plasma_window_management::Event,
        _: &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        use org_kde_plasma_window_management::Event;
        match event {
            Event::WindowWithUuid { uuid, .. } => {
                manager.get_window_by_uuid(uuid.clone(), queue, uuid);
            }
            // Before version 13 only the numeric id exists.
            Event::Window { id } => {
                manager.get_window(id, queue, format!("id-{id}"));
            }
            _ => {}
        }
    }
}

impl Dispatch<OrgKdePlasmaWindow, String> for State {
    fn event(
        state: &mut Self,
        window: &OrgKdePlasmaWindow,
        event: org_kde_plasma_window::Event,
        key: &String,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use org_kde_plasma_window::Event;
        let id = window.id().protocol_id();
        match event {
            Event::TitleChanged { title } => state.window(id, key).title = title,
            Event::AppIdChanged { app_id } => state.window(id, key).app_id = app_id,
            Event::StateChanged { flags } => state.window(id, key).flags = flags,
            Event::InitialState => {
                state.window(id, key);
            }
            Event::PidChanged { pid } => state.window(id, key).pid = Some(pid),
            Event::Unmapped => state.remove(id),
            _ => {}
        }
    }
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("wayland: {error}"))
}

/// The windows KWin lists; `None` when the compositor does not offer the
/// interface to this program.
pub(crate) fn list() -> io::Result<Option<WindowList>> {
    let connection = Connection::connect_to_env().map_err(failed)?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection).map_err(failed)?;
    let handle = queue.handle();
    let Ok(_manager) = globals.bind::<OrgKdePlasmaWindowManagement, _, _>(&handle, 1..=18, ())
    else {
        return Ok(None);
    };
    let mut state = State::default();
    for _ in 0..2 {
        queue.roundtrip(&mut state).map_err(failed)?;
    }
    let windows = state
        .order
        .iter()
        .filter_map(|id| state.windows.get(id))
        .filter(|(_, window)| window.flags & STATE_SKIPTASKBAR == 0)
        .map(|(key, window)| WindowEntry {
            id: clean_id(key),
            title: window.title.clone(),
            app: window.app_id.clone(),
            pid: window.pid,
            monitor: None,
            minimized: window.flags & STATE_MINIMIZED != 0,
            focused: window.flags & STATE_ACTIVE != 0,
        })
        .collect();
    Ok(Some(WindowList {
        windows,
        apps: Vec::new(),
        list_source: ListSource::KdePlasmaWindowManagement,
        limits: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KWin runs headless with `xdg-desktop-portal-kde`, a window with the
    /// title in `SE_SCREEN_TASK_KDE_TITLE` is open, and the test program has
    /// a desktop file with `X-KDE-Wayland-Interfaces=org_kde_plasma_window_
    /// management` and its absolute `Exec` path (assumption A6). Where KWin
    /// cannot be started the stage reports „nicht ausgeführt“.
    #[test]
    #[ignore = "needs headless KWin with the authorizing desktop file (SE_SCREEN_TASK_KDE=1)"]
    fn screen_task_kde_window_list() {
        assert!(
            std::env::var("SE_SCREEN_TASK_KDE").is_ok_and(|value| value == "1"),
            "SE_SCREEN_TASK_KDE=1 is not set"
        );
        let list = list()
            .expect("the Wayland connection")
            .expect("KWin offers org_kde_plasma_window_management to this program");
        assert_eq!(list.list_source, ListSource::KdePlasmaWindowManagement);
        assert!(!list.windows.is_empty(), "KWin listed no window");
        for window in &list.windows {
            assert!(!window.id.is_empty() && window.id.len() <= 64);
        }
        if let Ok(title) = std::env::var("SE_SCREEN_TASK_KDE_TITLE") {
            let found = list.windows.iter().find(|window| window.title == title);
            let found = found.expect("the test window is listed");
            assert!(!found.app.is_empty());
            assert!(!found.minimized);
        }
    }
}
