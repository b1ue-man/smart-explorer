//! The windows of a Wayland session from the compositor's own protocols
//! (SCR-M5, U19-7): `ext-foreign-toplevel-list-v1` first, then
//! `zwlr_foreign_toplevel_manager_v1`; the first one the registry offers is
//! used, never a merge of two. Each call opens a connection of its own,
//! binds, runs two round trips to collect the events and closes it. The list
//! is for display only: under Wayland a window cannot be opened as a source,
//! the user picks it in the portal dialog (`wayland_window_pick_on_host`).
use std::collections::HashMap;
use std::io;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self, ZwlrForeignToplevelManagerV1,
};

use crate::screen::core::window_list::{ListSource, WindowEntry, WindowList, MAX_ID_BYTES};

/// An id of visible ASCII only, at most 64 bytes: other characters become
/// `_`; an empty one becomes `unknown`.
pub(crate) fn clean_id(identifier: &str) -> String {
    let mut id: String = identifier
        .chars()
        .map(|c| if c.is_ascii_graphic() { c } else { '_' })
        .collect();
    if id.is_empty() {
        id.push_str("unknown");
    }
    id.truncate(MAX_ID_BYTES);
    id
}

/// What the events of one toplevel said.
#[derive(Default, Clone)]
struct Toplevel {
    title: String,
    app_id: String,
    identifier: Option<String>,
    minimized: bool,
    focused: bool,
}

#[derive(Default)]
struct State {
    toplevels: HashMap<u32, Toplevel>,
    /// Order of appearance, for ids of protocols without an identifier.
    order: Vec<u32>,
}

impl State {
    fn entry(&mut self, id: u32) -> &mut Toplevel {
        if !self.toplevels.contains_key(&id) {
            self.order.push(id);
        }
        self.toplevels.entry(id).or_default()
    }

    fn remove(&mut self, id: u32) {
        self.toplevels.remove(&id);
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

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.entry(toplevel.id().protocol_id());
        }
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_foreign_toplevel_handle_v1::Event;
        let id = handle.id().protocol_id();
        match event {
            Event::Title { title } => state.entry(id).title = title,
            Event::AppId { app_id } => state.entry(id).app_id = app_id,
            Event::Identifier { identifier } => state.entry(id).identifier = Some(identifier),
            Event::Closed => state.remove(id),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.entry(toplevel.id().protocol_id());
        }
    }

    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

/// `zwlr_foreign_toplevel_handle_v1.state` values.
const WLR_STATE_MINIMIZED: u32 = 1;
const WLR_STATE_ACTIVATED: u32 = 2;

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Event;
        let id = handle.id().protocol_id();
        match event {
            Event::Title { title } => state.entry(id).title = title,
            Event::AppId { app_id } => state.entry(id).app_id = app_id,
            Event::State { state: values } => {
                let entry = state.entry(id);
                let values: Vec<u32> = values
                    .chunks_exact(4)
                    .map(|bytes| u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                    .collect();
                entry.minimized = values.contains(&WLR_STATE_MINIMIZED);
                entry.focused = values.contains(&WLR_STATE_ACTIVATED);
            }
            Event::Closed => state.remove(id),
            _ => {}
        }
    }
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("wayland: {error}"))
}

/// The windows the compositor lists through `ext-foreign-toplevel-list-v1`
/// or the wlr protocol; `None` when it offers neither.
pub(crate) fn list() -> io::Result<Option<WindowList>> {
    let connection = Connection::connect_to_env().map_err(failed)?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection).map_err(failed)?;
    let handle = queue.handle();
    let mut state = State::default();
    // The bound object stays alive until the events are in.
    let (source, _bound): (ListSource, Box<dyn std::any::Any>) =
        if let Ok(list) = globals.bind::<ExtForeignToplevelListV1, _, _>(&handle, 1..=1, ()) {
            (ListSource::ExtForeignToplevel, Box::new(list))
        } else if let Ok(manager) =
            globals.bind::<ZwlrForeignToplevelManagerV1, _, _>(&handle, 1..=3, ())
        {
            (ListSource::WlrForeignToplevel, Box::new(manager))
        } else {
            return Ok(None);
        };
    for _ in 0..2 {
        queue.roundtrip(&mut state).map_err(failed)?;
    }
    let windows = state
        .order
        .iter()
        .filter_map(|id| state.toplevels.get(id).map(|toplevel| (id, toplevel)))
        .map(|(id, toplevel)| WindowEntry {
            id: match &toplevel.identifier {
                Some(identifier) => clean_id(identifier),
                None => format!("wlr-{id}"),
            },
            title: toplevel.title.clone(),
            app: toplevel.app_id.clone(),
            app_id: (!toplevel.app_id.is_empty()).then(||toplevel.app_id.clone()),
            icon: None,
            pid: None,
            monitor: None,
            minimized: toplevel.minimized,
            focused: toplevel.focused,
        })
        .collect();
    Ok(Some(WindowList {
        windows,
        apps: Vec::new(),
        list_source: source,
        limits: Vec::new(),
    }))
}
