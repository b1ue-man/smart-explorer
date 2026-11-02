//! DSK-M4 view model of the surface: a new configuration of the shell
//! applies at once, and the vault group stays hidden without the background
//! service (the feed never asks for vaults then).
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::{Outbox, ViewModel};
use crate::app::desktop_surface_actions::{clip_text, parse_clip_text};
use crate::desktop::core::config::{Background, BackgroundKind, DesktopConfig, SystemItems};
use crate::desktop::core::items::{self, DesktopFile};
use crate::desktop::core::layout::{Cell, ItemId, MonitorLayout};
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage, SurfaceModel};
use crate::desktop::core::state::Platform;
use crate::desktop::core::tiles::{ShareState, TileModel, VaultTile};
use crate::desktop::os::feed_sources::{DesktopFolder, DeviceTreeView};
use crate::desktop::os::model_feed::{FeedRound, FeedSources, NO_SERVICE};
use crate::desktop::os::wallpaper_image::{spec_for, WallpaperSpec};

const SITE: &str = "0123456789abcdef0123456789abcdef";

#[derive(Clone, Default)]
struct Sent(Rc<RefCell<Vec<SurfaceMessage>>>);

impl Outbox for Sent {
    fn send(&self, message: SurfaceMessage) -> bool {
        self.0.borrow_mut().push(message);
        true
    }
}

fn config() -> DesktopConfig {
    DesktopConfig::initial(SystemItems::initial(None, None))
}

fn view() -> ViewModel {
    let folders = vec![DesktopFolder {
        path: PathBuf::from("/home/nutzer/Schreibtisch"),
        public: false,
        launchers: false,
    }];
    ViewModel::new("mon-a", Box::new(Sent::default()), true, config(), folders)
}

fn file(name: &str) -> DesktopFile {
    DesktopFile {
        name: name.into(),
        is_dir: false,
        public: false,
        launcher: false,
        hidden: false,
    }
}

/// A model with a vault, a file and "Alle Bildschirme"; `vaults: None`
/// without data.
fn model(share_state: ShareState, vaults: Option<Vec<VaultTile>>) -> SurfaceModel {
    let mut layout = MonitorLayout::new("mon-a", 6).unwrap();
    let placed = [
        ItemId::Vault { site: SITE.into() },
        ItemId::File {
            name: "notiz.txt".into(),
        },
        ItemId::AllScreens,
    ];
    for (row, item) in (0u32..).zip(placed) {
        layout.cells.insert(item, Cell { col: 0, row });
    }
    SurfaceModel {
        tiles: TileModel {
            share_state,
            profiles_error: None,
            devices: Vec::new(),
            rooms: Vec::new(),
            vaults,
            empty_actions: Vec::new(),
        },
        files: vec![file("notiz.txt")],
        system_items: Vec::new(),
        screens: true,
        layout,
    }
}

fn locked_vault() -> Vec<VaultTile> {
    vec![VaultTile {
        site: SITE.into(),
        label: "Privat".into(),
        state: "locked".into(),
    }]
}

fn shown(view: &ViewModel) -> Vec<ItemId> {
    view.visible().into_iter().map(|(item, _)| item).collect()
}

#[test]
fn desktop_task_config_reload() {
    let mut view = view();
    let model = model(ShareState::Live, Some(locked_vault()));
    assert!(view.absorb(ShellMessage::Model {
        rev: 1,
        monitor: None,
        model: Box::new(model),
    }));
    assert_eq!(shown(&view).len(), 3);
    // `se desktop config --vaults off --desktop-files off`: the shell sends
    // `config`, the surface hides the groups before its next model.
    let mut hidden = config();
    hidden.groups.vaults = false;
    hidden.groups.desktop_files = false;
    assert!(view.absorb(ShellMessage::Config {
        config: hidden.clone()
    }));
    assert_eq!(shown(&view), [ItemId::AllScreens]);
    assert!(!view.absorb(ShellMessage::Config { config: hidden }));
    // `--background color:#102030` changes what the wallpaper paints at once.
    let mut colored = config();
    colored.background = Background {
        kind: BackgroundKind::Color,
        value: Some("#102030".into()),
    };
    assert!(view.absorb(ShellMessage::Config { config: colored }));
    assert_eq!(shown(&view).len(), 3);
    assert_eq!(
        spec_for(&view.config().background, None),
        WallpaperSpec::Color([0x10, 0x20, 0x30])
    );
    // The file's path comes from the desktop folder of the surface.
    let notiz = file("notiz.txt");
    assert_eq!(
        view.file_path(&notiz),
        Some(PathBuf::from("/home/nutzer/Schreibtisch/notiz.txt"))
    );
}

type Snapshot = crate::daemon::ShareWorkerSnapshot;
type Answer = Result<Option<serde_json::Value>, String>;

fn sources(peek_running: bool, vault_reads: Arc<AtomicUsize>, vaults_fail: bool) -> FeedSources {
    let mut sources = FeedSources::system();
    sources.peek = Arc::new(move || -> Result<Option<Snapshot>, String> {
        if peek_running {
            Ok(Some(Snapshot::default()))
        } else {
            Ok(None)
        }
    });
    sources.ensure_worker =
        Arc::new(|| -> Result<(), String> { Err("im Test nie gestartet".into()) });
    sources.devices = Arc::new(|| -> Answer { Ok(None) });
    sources.devices_watch = Arc::new(|_since: u64| -> Answer { Ok(None) });
    sources.vaults = Arc::new(move || -> Result<Option<Vec<VaultTile>>, String> {
        vault_reads.fetch_add(1, Ordering::SeqCst);
        if vaults_fail {
            Err("Hintergrunddienst kennt diese Abfrage nicht".into())
        } else {
            Ok(Some(Vec::new()))
        }
    });
    sources.desktop_folders = Arc::new(Vec::<DesktopFolder>::new);
    sources
}

#[test]
fn desktop_task_vault_group_without_daemon_hidden() {
    // Without the service the feed never asks for vaults: no vault group.
    let reads = Arc::new(AtomicUsize::new(0));
    let mut round = FeedRound::new(sources(false, reads.clone(), false));
    let state = round.step(&DeviceTreeView::NotRunning).unwrap();
    assert_eq!(state.tiles.vaults, None);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    let items = items::surface_items(Platform::Linux, &config(), &state.files, &state.tiles);
    assert!(!items
        .iter()
        .any(|item| matches!(item, ItemId::Vault { .. })));
    // A vault placed earlier stays in the arrangement but is not shown; the
    // surface names why the group is missing.
    let mut view = view();
    view.absorb(ShellMessage::Model {
        rev: 1,
        monitor: None,
        model: Box::new(model(state.tiles.share_state.clone(), None)),
    });
    assert!(!shown(&view).contains(&ItemId::Vault { site: SITE.into() }));
    let line = format!("Tresore: {NO_SERVICE}");
    assert_eq!(view.vault_line(), Some(line.clone()));
    assert!(view.status_lines().contains(&line));
    assert!(view.offers_connect());
    // A running service that refuses `Vault::Status`: state unknown, and the
    // next read waits for its retry.
    let reads = Arc::new(AtomicUsize::new(0));
    let mut round = FeedRound::new(sources(true, reads.clone(), true));
    let state = round.step(&DeviceTreeView::NotRunning).unwrap();
    assert_eq!(state.tiles.vaults, None);
    assert_eq!(state.tiles.share_state, ShareState::Live);
    let _ = round.step(&DeviceTreeView::NotRunning);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    let mut view_live = self::view();
    view_live.absorb(ShellMessage::Model {
        rev: 2,
        monitor: None,
        model: Box::new(model(ShareState::Live, None)),
    });
    assert_eq!(
        view_live.vault_line().as_deref(),
        Some("Tresore: Zustand unbekannt")
    );
    assert!(!view_live.offers_connect());
    // With vault data the group shows and no line is needed.
    let mut with_vaults = self::view();
    with_vaults.absorb(ShellMessage::Model {
        rev: 3,
        monitor: None,
        model: Box::new(model(ShareState::Live, Some(locked_vault()))),
    });
    assert!(shown(&with_vaults).contains(&ItemId::Vault { site: SITE.into() }));
    assert_eq!(with_vaults.vault_line(), None);
}

#[test]
fn surface_clipboard_text_names_files() {
    // `parse_clip_text` keeps only paths absolute on this host; a Unix path
    // has no drive on Windows.
    #[cfg(windows)]
    let folder = r"C:\Users\nutzer\Desktop";
    #[cfg(not(windows))]
    let folder = "/home/nutzer/Schreibtisch";
    let files = [
        PathBuf::from(folder).join("a.txt"),
        PathBuf::from(folder).join("b c.txt"),
    ];
    let text = clip_text(&files, true);
    assert_eq!(parse_clip_text(&text), Some((files.to_vec(), true)));
    assert_eq!(
        parse_clip_text(&clip_text(&files, false)),
        Some((files.to_vec(), false))
    );
    assert_eq!(parse_clip_text("irgendein Text"), None);
    assert_eq!(parse_clip_text("x-se-desktop-copy\nrelativ.txt"), None);
}
