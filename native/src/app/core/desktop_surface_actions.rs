//! What the surface's handlings start (DSK-M4): windows and pages through
//! `open_target` (detached processes of the GUI program), files and folders
//! through the app's `open_local_path` (Linux: "Öffnen mit …" and starters
//! through the portal, `platform::open_desktop_path`), file actions of the desktop folder
//! through `desktop_ops` in a thread of their own, and the file clipboard.
//! Windows: the system's file clipboard (Explorer reads it). Elsewhere the
//! paths go as text with a marker onto the clipboard, so a Ctrl+V on another
//! surface finds them; each surface also keeps its own copy for the menu
//! entry "Einfügen". The tests record instead of starting anything.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

use crate::app::folder_conflict_ui;
use crate::app::platform_helpers::{
    clipboard_file_ops_supported, open_local_path, read_clipboard_files, write_clipboard_files,
    ClipboardEffect,
};
use crate::app::support_paths::OpenMode;
use crate::desktop::os::desktop_ops;
use crate::desktop::os::open_target::{self, OpenRequest};
use crate::types::CopyMode;

/// First line of the clipboard text of a copy or a cut in a surface.
const CLIP_COPY: &str = "x-se-desktop-copy";
const CLIP_CUT: &str = "x-se-desktop-cut";

/// A file action in the desktop folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DesktopOp {
    Rename {
        folder: PathBuf,
        name: String,
        new_name: String,
    },
    NewFolder {
        folder: PathBuf,
    },
    Recycle {
        folder: PathBuf,
        names: Vec<String>,
    },
    /// Paste or drop: copy, or move after a cut.
    Transfer {
        sources: Vec<PathBuf>,
        folder: PathBuf,
        cut: bool,
    },
}

pub(crate) trait SurfaceActions {
    fn open(&mut self, request: OpenRequest);
    fn run(&mut self, op: DesktopOp);
    /// "Kopieren"/"Ausschneiden" (Ctrl+C, Ctrl+X).
    fn set_clipboard(&mut self, ctx: &egui::Context, files: Vec<PathBuf>, cut: bool);
    /// The files to paste and whether they were cut; `pasted` is the text a
    /// Ctrl+V brought along.
    fn clipboard(&mut self, pasted: Option<&str>) -> Option<(Vec<PathBuf>, bool)>;
    /// Messages of finished handlings (errors), each once.
    fn take_notices(&mut self) -> Vec<String>;
}

/// The clipboard text of a surface copy or cut.
pub(crate) fn clip_text(files: &[PathBuf], cut: bool) -> String {
    let mut text = String::from(if cut { CLIP_CUT } else { CLIP_COPY });
    for file in files {
        text.push('\n');
        text.push_str(&file.to_string_lossy());
    }
    text
}

/// The files of a surface's clipboard text; other text is no file list.
pub(crate) fn parse_clip_text(text: &str) -> Option<(Vec<PathBuf>, bool)> {
    let mut lines = text.lines();
    let cut = match lines.next()? {
        CLIP_COPY => false,
        CLIP_CUT => true,
        _ => return None,
    };
    let files: Vec<PathBuf> = lines
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect();
    (!files.is_empty()).then_some((files, cut))
}

/// The production actions of a surface process.
pub(crate) struct SystemActions {
    gui: Result<PathBuf, String>,
    ctx: egui::Context,
    notices: Sender<String>,
    inbox: Receiver<String>,
    /// The surface's own clipboard (menu entry "Einfügen").
    own: Option<(Vec<PathBuf>, bool)>,
    folders: std::collections::VecDeque<folder_conflict_ui::Admission>,
}

impl SystemActions {
    pub(crate) fn new(ctx: egui::Context) -> SystemActions {
        let (notices, inbox) = mpsc::channel();
        SystemActions {
            gui: std::env::current_exe().map_err(|error| error.to_string()),
            ctx,
            notices,
            inbox,
            own: None,
            folders: std::collections::VecDeque::new(),
        }
    }

    fn notice(&self, text: String) {
        let _ = self.notices.send(text);
    }

    fn run_transfer_job(&mut self, job: crate::transfer::TransferJob) {
        let consume_cut = job.mode == CopyMode::Move && self.own.as_ref().is_some_and(|(paths, cut)| {
            if !cut { return false; }
            let own: Result<Vec<String>, _> = paths.iter().map(|path| crate::copy::native_path_text(path)).collect();
            matches!((&job.items, own), (crate::transfer::JobItems::Roots { paths, .. }, Ok(own)) if *paths == own)
        });
        let notices = self.notices.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("se-desktop-transfer".into())
            .spawn(move || {
                for (path, error) in desktop_ops::run_transfer_job(job).errors {
                    let _ = notices.send(format!("{path}: {error}"));
                }
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => {
                if consume_cut {
                    self.own = None;
                }
            }
            Err(error) => self.notice(format!("Übertragung nicht gestartet: {error}")),
        }
    }
}

fn open_path(path: &Path, choose: bool) {
    // Linux (U34-6, U34-7): "Öffnen mit …" and a starter go through the
    // OpenURI portal or `gio launch` by the rule of the desktop; everything
    // else stays with `xdg-open`.
    #[cfg(target_os = "linux")]
    if crate::desktop::os::platform::open_desktop_path(path, choose) {
        return;
    }
    let mode = if choose {
        OpenMode::With
    } else {
        OpenMode::Default
    };
    open_local_path(&path.to_string_lossy(), mode);
}

/// One file action; its error messages.
fn perform(op: DesktopOp) -> Vec<String> {
    match op {
        DesktopOp::Rename {
            folder,
            name,
            new_name,
        } => desktop_ops::rename(&folder, &name, &new_name)
            .err()
            .into_iter()
            .collect(),
        DesktopOp::NewFolder { folder } => {
            desktop_ops::new_folder(&folder).err().into_iter().collect()
        }
        DesktopOp::Recycle { folder, names } => {
            desktop_ops::recycle(&folder, &names, &desktop_ops::system_recycle)
                .into_iter()
                .map(|(name, error)| format!("„{name}“ nicht im Papierkorb: {error}"))
                .collect()
        }
        DesktopOp::Transfer {
            sources,
            folder,
            cut,
        } => {
            let mode = if cut { CopyMode::Move } else { CopyMode::Copy };
            desktop_ops::transfer(&sources, &folder, mode)
                .errors
                .into_iter()
                .map(|(path, error)| format!("{path}: {error}"))
                .collect()
        }
    }
}

impl SurfaceActions for SystemActions {
    fn open(&mut self, request: OpenRequest) {
        let gui = match &self.gui {
            Ok(gui) => gui.clone(),
            Err(error) => {
                self.notice(format!("Smart Explorer nicht gefunden: {error}"));
                return;
            }
        };
        if let Err(error) =
            open_target::launch(&request, &gui, &open_target::spawn_detached, &open_path)
        {
            self.notice(error);
        }
    }

    fn run(&mut self, op: DesktopOp) {
        if let DesktopOp::Transfer {
            sources,
            folder,
            cut,
        } = &op
        {
            let mode = if *cut { CopyMode::Move } else { CopyMode::Copy };
            let (job, errors) = desktop_ops::transfer_job(sources, folder, mode);
            for (path, error) in errors {
                self.notice(format!("{path}: {error}"));
            }
            if let Some(job) = job {
                match folder_conflict_ui::Admission::new(job) {
                    Ok(admission) => self.folders.push_back(admission),
                    Err(error) => self.notice(error),
                }
            }
            self.ctx.request_repaint();
            return;
        }
        let notices = self.notices.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("se-desktop-op".into())
            .spawn(move || {
                for notice in perform(op) {
                    let _ = notices.send(notice);
                }
                ctx.request_repaint();
            });
        if let Err(error) = spawned {
            self.notice(format!("Vorgang nicht gestartet: {error}"));
        }
    }

    fn set_clipboard(&mut self, ctx: &egui::Context, files: Vec<PathBuf>, cut: bool) {
        if files.is_empty() {
            return;
        }
        if clipboard_file_ops_supported() {
            let paths: Vec<String> = files
                .iter()
                .map(|file| file.to_string_lossy().into_owned())
                .collect();
            let effect = if cut {
                ClipboardEffect::Move
            } else {
                ClipboardEffect::Copy
            };
            if let Err(error) = write_clipboard_files(&paths, effect) {
                self.notice(format!("Zwischenablage: {error}"));
            }
        } else {
            ctx.copy_text(clip_text(&files, cut));
        }
        self.own = Some((files, cut));
    }

    fn clipboard(&mut self, pasted: Option<&str>) -> Option<(Vec<PathBuf>, bool)> {
        if clipboard_file_ops_supported() {
            if let Ok(Some((paths, cut))) = read_clipboard_files() {
                return Some((paths.into_iter().map(PathBuf::from).collect(), cut));
            }
        }
        if let Some(found) = pasted.and_then(parse_clip_text) {
            return Some(found);
        }
        self.own.clone()
    }

    fn take_notices(&mut self) -> Vec<String> {
        if let Some(result) = self
            .folders
            .front_mut()
            .and_then(|admission| admission.show(&self.ctx))
        {
            self.folders.pop_front();
            match result {
                Ok(Some(job)) => self.run_transfer_job(job),
                Ok(None) => {}
                Err(error) => self.notice(format!("Ordnerkonflikte prüfen: {error}")),
            }
        }
        self.inbox.try_iter().collect()
    }
}
