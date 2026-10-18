//! DSK-M4 GUI harness of the surface (`SMART_EXPLORER_GUI_TASK=1`, isolated
//! app data, one test thread, `--include-ignored`): first the view inside
//! this process, driven with pointer and keys like a user, then the surface
//! process in its window test mode (`desktop_surface_window_tests.rs`).
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use eframe::egui::{self, Pos2, Rect, Vec2};

use super::{Outbox, ViewModel};
use crate::app::desktop_surface_actions::{DesktopOp, SurfaceActions};
use crate::app::desktop_surface_grid_ui::{cell_rect, show_surface, SurfaceState};
use crate::app::desktop_surface_host::{NativeWindow, SurfaceHost};
use crate::app::start_request::{StartPage, StartRequest};
use crate::desktop::core::config::{DesktopConfig, SystemItems};
use crate::desktop::core::items::DesktopFile;
use crate::desktop::core::layout::{Cell, ItemId, LayoutOp, MonitorLayout};
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage, SurfaceModel};
use crate::desktop::core::tiles::{
    self, DeviceTile, RoomGroup, ShareState, TileCode, TileModel, TileOpen, VaultTile,
};
use crate::desktop::os::feed_sources::DesktopFolder;
use crate::desktop::os::open_target::OpenRequest;
use crate::desktop::os::Monitor;
use crate::share::PeerOpenTarget;

const SITE: &str = "0123456789abcdef0123456789abcdef";
const SCREEN: Vec2 = Vec2::new(1024.0, 768.0);
pub(super) const MONITOR: &str = "task-a";

#[derive(Clone, Default)]
struct Sent(Rc<RefCell<Vec<SurfaceMessage>>>);

impl Outbox for Sent {
    fn send(&self, message: SurfaceMessage) -> bool {
        self.0.borrow_mut().push(message);
        true
    }
}

impl Sent {
    fn layout_ops(&self) -> Vec<LayoutOp> {
        self.0
            .borrow()
            .iter()
            .filter_map(|message| match message {
                SurfaceMessage::Layout { op } => Some(op.clone()),
                _ => None,
            })
            .collect()
    }
}

#[derive(Default)]
struct Recorder {
    opened: Vec<OpenRequest>,
    ops: Vec<DesktopOp>,
    clipboard: Option<(Vec<PathBuf>, bool)>,
}

impl SurfaceActions for Recorder {
    fn open(&mut self, request: OpenRequest) {
        self.opened.push(request);
    }

    fn run(&mut self, op: DesktopOp) {
        self.ops.push(op);
    }

    fn set_clipboard(&mut self, _ctx: &egui::Context, files: Vec<PathBuf>, cut: bool) {
        self.clipboard = Some((files, cut));
    }

    fn clipboard(&mut self, _pasted: Option<&str>) -> Option<(Vec<PathBuf>, bool)> {
        self.clipboard.clone()
    }

    fn take_notices(&mut self) -> Vec<String> {
        Vec::new()
    }
}

struct TestHost {
    monitors: Vec<Monitor>,
}

impl SurfaceHost for TestHost {
    fn monitor_key(&self) -> &str {
        MONITOR
    }

    fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    fn work_area(&self) -> Option<[i32; 4]> {
        None
    }

    fn native_window(&self) -> Option<NativeWindow> {
        None
    }

    fn request_repaint(&self) {}
}

fn device(target: PeerOpenTarget, code: TileCode) -> DeviceTile {
    let location = format!("{}/", target.endpoint_prefix());
    let page = match &target {
        PeerOpenTarget::Direct { contact_id } => format!("share-device:{contact_id}"),
        PeerOpenTarget::RoomDevice { room_id, .. } => format!("share-room:{room_id}"),
    };
    let open = if code.opens_files() {
        TileOpen::Location(location.clone())
    } else {
        TileOpen::Page(page)
    };
    DeviceTile {
        key: tiles::device_key(&target),
        target,
        name: "Gerät".into(),
        code,
        status_text: String::new(),
        last_seen: Some(1_000),
        lan: true,
        online: true,
        write: None,
        location,
        device_id: Some("geraet-1".into()),
        folder_path: None,
        open,
    }
}

fn direct(contact_id: &str) -> PeerOpenTarget {
    PeerOpenTarget::Direct {
        contact_id: contact_id.into(),
    }
}

/// Every kind of tile, five rows (the surface asks for six).
pub(super) fn model() -> SurfaceModel {
    let connected = device(direct("c1"), TileCode::Connected);
    let conflict = device(direct("c2"), TileCode::IdentityConflict);
    let members = [TileCode::Connecting, TileCode::Offline, TileCode::Failed]
        .into_iter()
        .enumerate()
        .map(|(index, code)| {
            let target = PeerOpenTarget::RoomDevice {
                room_id: "raum-1".into(),
                device_id: format!("d{index}"),
            };
            device(target, code)
        })
        .collect();
    let mut layout = MonitorLayout::new(MONITOR, 5).unwrap();
    let placed = [
        (
            ItemId::Device {
                key: connected.key.clone(),
            },
            0,
            0,
        ),
        (
            ItemId::Device {
                key: conflict.key.clone(),
            },
            0,
            1,
        ),
        (
            ItemId::Room {
                room_id: "raum-1".into(),
            },
            0,
            2,
        ),
        (ItemId::Vault { site: SITE.into() }, 0, 3),
        (
            ItemId::File {
                name: "notiz.txt".into(),
            },
            0,
            4,
        ),
        (ItemId::AllScreens, 1, 0),
    ];
    for (item, col, row) in placed {
        layout.cells.insert(item, Cell { col, row });
    }
    SurfaceModel {
        tiles: TileModel {
            share_state: ShareState::Live,
            profiles_error: None,
            devices: vec![connected, conflict],
            rooms: vec![RoomGroup {
                room_id: "raum-1".into(),
                name: "Büro".into(),
                page: "share-room:raum-1".into(),
                members,
            }],
            vaults: Some(vec![VaultTile {
                site: SITE.into(),
                label: "Privat".into(),
                state: "locked".into(),
            }]),
            empty_actions: Vec::new(),
        },
        files: vec![DesktopFile {
            name: "notiz.txt".into(),
            is_dir: false,
            public: false,
            launcher: false,
            hidden: false,
        }],
        system_items: Vec::new(),
        screens: true,
        layout,
    }
}

pub(super) fn config() -> DesktopConfig {
    DesktopConfig::initial(SystemItems::initial(None, None))
}

struct Driver {
    ctx: egui::Context,
    time: f64,
    view: ViewModel,
    state: SurfaceState,
    host: TestHost,
    actions: Recorder,
}

impl Driver {
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let Driver {
            ctx,
            view,
            state,
            host,
            actions,
            ..
        } = self;
        let _ = ctx.run(input, |ctx| show_surface(ctx, view, state, host, actions));
    }

    fn center(cell: Cell) -> Pos2 {
        cell_rect(Rect::from_min_size(Pos2::ZERO, SCREEN), cell).center()
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click(&mut self, cell: Cell) {
        let pos = Driver::center(cell);
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![Driver::button(pos, true)]);
        self.frame(vec![Driver::button(pos, false)]);
    }

    fn double_click(&mut self, cell: Cell) {
        let pos = Driver::center(cell);
        self.click(cell);
        self.frame(vec![Driver::button(pos, true)]);
        self.frame(vec![Driver::button(pos, false)]);
    }

    fn key(&mut self, key: egui::Key) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
    }

    fn drag(&mut self, from: Cell, to: Cell) {
        let (start, end) = (Driver::center(from), Driver::center(to));
        self.frame(vec![egui::Event::PointerMoved(start)]);
        self.frame(vec![Driver::button(start, true)]);
        self.frame(vec![egui::Event::PointerMoved(start + (end - start) / 2.0)]);
        self.frame(vec![egui::Event::PointerMoved(end)]);
        self.frame(vec![Driver::button(end, false)]);
    }

    fn last_opened(&self) -> Option<&OpenRequest> {
        self.actions.opened.last()
    }
}

fn in_process_view() {
    let desktop = PathBuf::from("/home/nutzer/Schreibtisch");
    let folders = vec![DesktopFolder {
        path: desktop.clone(),
        public: false,
        launchers: false,
    }];
    let sent = Sent::default();
    let mut view = ViewModel::new(MONITOR, Box::new(sent.clone()), true, config(), folders);
    view.absorb(ShellMessage::Model {
        rev: 1,
        model: Box::new(model()),
    });
    let monitors = vec![
        Monitor {
            key: MONITOR.into(),
            primary: true,
            rect: Some([0, 0, 1024, 768]),
        },
        Monitor {
            key: "task-b".into(),
            primary: false,
            rect: Some([1024, 0, 2048, 768]),
        },
    ];
    let mut driver = Driver {
        ctx: egui::Context::default(),
        time: 0.0,
        view,
        state: SurfaceState::new(false),
        host: TestHost { monitors },
        actions: Recorder::default(),
    };
    // Every tile drawn; the work area holds six rows.
    driver.frame(Vec::new());
    let grid = LayoutOp::Grid {
        monitor: MONITOR.into(),
        rows: 6,
    };
    assert!(sent.layout_ops().contains(&grid), "{:?}", sent.layout_ops());
    // Double click: a device in a se window, the page after an identity
    // change, a vault at its root.
    driver.double_click(Cell { col: 0, row: 0 });
    let location = StartRequest::OpenLocation {
        locator: "share://direct/c1/".into(),
    };
    assert_eq!(driver.last_opened(), Some(&OpenRequest::Window(location)));
    driver.double_click(Cell { col: 0, row: 1 });
    let page = StartRequest::OpenPage {
        page: StartPage::ShareDevice {
            contact_id: "c2".into(),
        },
    };
    assert_eq!(driver.last_opened(), Some(&OpenRequest::Window(page)));
    driver.double_click(Cell { col: 0, row: 3 });
    let vault = StartRequest::OpenLocation {
        locator: format!("vault://{SITE}/"),
    };
    assert_eq!(driver.last_opened(), Some(&OpenRequest::Window(vault)));
    // A click opens the room's member devices; Esc closes them.
    driver.click(Cell { col: 0, row: 2 });
    assert_eq!(driver.state.open_room.as_deref(), Some("raum-1"));
    driver.frame(Vec::new());
    driver.key(egui::Key::Escape);
    assert_eq!(driver.state.open_room, None);
    // F2 renames the file, Esc cancels; Del moves it to the recycle bin.
    driver.click(Cell { col: 0, row: 4 });
    driver.key(egui::Key::F2);
    assert!(driver.state.rename.is_some());
    driver.key(egui::Key::Escape);
    assert!(driver.state.rename.is_none());
    driver.click(Cell { col: 0, row: 4 });
    driver.key(egui::Key::Delete);
    let recycle = DesktopOp::Recycle {
        folder: desktop.clone(),
        names: vec!["notiz.txt".into()],
    };
    assert_eq!(driver.actions.ops.last(), Some(&recycle));
    // Ctrl+C and Ctrl+V of a file: into the desktop folder.
    driver.click(Cell { col: 0, row: 4 });
    driver.frame(vec![egui::Event::Copy]);
    let copied = vec![desktop.join("notiz.txt")];
    assert_eq!(driver.actions.clipboard, Some((copied.clone(), false)));
    driver.frame(vec![egui::Event::Paste("notiz".into())]);
    let paste = DesktopOp::Transfer {
        sources: copied,
        folder: desktop,
        cut: false,
    };
    assert_eq!(driver.actions.ops.last(), Some(&paste));
    // Dragging arranges: "Alle Bildschirme" to column 2, row 3.
    driver.drag(Cell { col: 1, row: 0 }, Cell { col: 2, row: 3 });
    let moved = LayoutOp::Move {
        item: ItemId::AllScreens,
        monitor: MONITOR.into(),
        col: 2,
        row: 3,
    };
    assert!(
        sent.layout_ops().contains(&moved),
        "{:?}",
        sent.layout_ops()
    );
    assert!(driver.state.selected.len() <= 1);
}

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint"]
fn desktop_task_surface_windowed_gui_harness() {
    assert_eq!(std::env::var("SMART_EXPLORER_GUI_TASK").as_deref(), Ok("1"));
    in_process_view();
    super::window_tests::windowed_process();
}
