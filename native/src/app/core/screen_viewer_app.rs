//! The window of the desktop viewer (SCR-M7, U25-2/3/6): `smart_explorer
//! --screen-view` starts it with one device, `--screen-grid` with the group
//! view; both switch to the other in the same window, over the same
//! connection where the host allows it (`screen_pool`). The window draws
//! with wgpu: `AutoVsync` and one frame of latency (decision C4); „Niedrigste
//! Latenz“ chooses `AutoNoVsync`, which only the next start of the viewer
//! applies. `Mailbox` is never set: eframe does not ask the surface first,
//! and a surface without it ends the program.
use std::time::Duration;

use eframe::egui::{self, ViewportCommand};
use eframe::egui_wgpu::{self, wgpu};

use super::screen_devices::GridScope;
use super::screen_input_keys::{local_keys, LocalKey};
use super::screen_pool::{preview_edge, to_tile, to_view, DEFAULT_PREVIEW_INTERVAL_MS};
use super::screen_prefs::ViewerPrefs;
use super::screen_session::Backend;
use super::screen_stats_overlay::stats_lines;
use super::screen_target::ScreenTarget;
use super::screen_tile_grid::ScreenTileGrid;
use super::screen_toolbar_ui::{self, ToolbarAction, ToolbarInput};
use super::screen_view_widget::ViewParams;
use super::screen_viewer_view::{device_list, Common, ViewState};
use super::screen_wgpu_yuv;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;

/// How the viewer starts.
#[derive(Clone)]
pub(crate) enum Start {
    View(ScreenTarget),
    Grid(GridScope),
}

enum Mode {
    View(Box<ViewState>),
    Grid(Box<ScreenTileGrid>),
}

enum Transition {
    Grid,
    View(ScreenTarget),
    Fullscreen,
    Close,
}

/// The wgpu settings of the viewer: with the vertical blank (no tearing)
/// and at most one frame queued, or without the blank for the lowest
/// latency. The settings take effect when the window opens.
pub(crate) fn wgpu_configuration(low_latency: bool) -> egui_wgpu::WgpuConfiguration {
    egui_wgpu::WgpuConfiguration {
        present_mode: if low_latency {
            wgpu::PresentMode::AutoNoVsync
        } else {
            wgpu::PresentMode::AutoVsync
        },
        desired_maximum_frame_latency: Some(1),
        ..Default::default()
    }
}

pub(crate) struct ScreenViewerApp {
    common: Common,
    mode: Mode,
    /// The group view „Alle Geräte“ opens.
    scope: GridScope,
    fullscreen: bool,
    want_fullscreen: bool,
    toolbar_open: bool,
    toolbar_height: f32,
    title: String,
}

impl ScreenViewerApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, start: Start, prefs: ViewerPrefs) -> Self {
        let ctx = &cc.egui_ctx;
        let gpu = cc
            .wgpu_render_state
            .as_ref()
            .is_some_and(screen_wgpu_yuv::install);
        let backend = Backend::production(ctx);
        let devices = super::screen_devices::Devices::start(ctx);
        let common = Common {
            backend: backend.clone(),
            devices: devices.clone(),
            prefs,
            gpu,
            stats_on: false,
            low_latency_changed: false,
        };
        let (mode, scope, want_fullscreen) = match start {
            Start::View(target) => {
                let fullscreen = target.fullscreen;
                let mode = Mode::View(Box::new(ViewState::new(target, gpu)));
                (mode, GridScope::AllDevices, fullscreen)
            }
            Start::Grid(scope) => {
                let grid = ScreenTileGrid::new(scope.clone(), backend, devices);
                (Mode::Grid(Box::new(grid)), scope, false)
            }
        };
        Self {
            common,
            mode,
            scope,
            fullscreen: false,
            want_fullscreen,
            toolbar_open: false,
            toolbar_height: 0.0,
            title: String::new(),
        }
    }

    fn display_size(ctx: &egui::Context) -> FrameSize {
        let size = ctx.screen_rect().size() * ctx.pixels_per_point();
        FrameSize::new(
            size.x.round().max(1.0) as u32,
            size.y.round().max(1.0) as u32,
        )
    }

    fn set_title(&mut self, ctx: &egui::Context) {
        let title = match &self.mode {
            Mode::View(view) => format!("Bildschirm – {}", view.device_name(&self.common)),
            Mode::Grid(_) => "Bildschirme".to_string(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn show_grid(&mut self, ctx: &egui::Context) -> Vec<Transition> {
        let Mode::Grid(grid) = &mut self.mode else {
            return Vec::new();
        };
        let mut chosen = None;
        egui::CentralPanel::default().show(ctx, |ui| chosen = grid.show(ui));
        if local_keys(&ctx.input(|input| input.events.clone())).contains(&LocalKey::Fullscreen) {
            return vec![Transition::Fullscreen];
        }
        chosen.map(Transition::View).into_iter().collect()
    }

    fn show_view(&mut self, ctx: &egui::Context) -> Vec<Transition> {
        let Mode::View(view) = &mut self.mode else {
            return Vec::new();
        };
        let common = &mut self.common;
        let mut out = Vec::new();
        let display = Self::display_size(ctx);
        view.ensure_session(common, display);
        if self.fullscreen {
            if let Some(y) = ctx.input(|input| input.pointer.hover_pos().map(|pos| pos.y)) {
                if y < 4.0 {
                    self.toolbar_open = true;
                } else if y > self.toolbar_height + 12.0 {
                    self.toolbar_open = false;
                }
            }
        }
        let status = view.status_text();
        let name = view.device_name(common);
        let denied = view
            .session
            .as_ref()
            .is_some_and(|s| s.info().control_denied);
        if denied && view.control_on {
            view.control_on = false;
        }
        let mut actions: Vec<ToolbarAction> = Vec::new();
        if !self.fullscreen || self.toolbar_open {
            let devices = device_list(&common.devices);
            let _ = egui::TopBottomPanel::top("screen_toolbar").show(ctx, |ui| {
                let info = view.session.as_ref().map(|session| session.info());
                let holder = info.as_ref().and_then(|info| {
                    info.controller
                        .as_ref()
                        .filter(|_| !view.control_on || info.control_denied)
                        .map(|holder| holder.device_name.clone())
                });
                let input = ToolbarInput {
                    devices: &devices,
                    current: &view.target,
                    current_name: &name,
                    info: info.as_deref(),
                    source: view.source.as_ref(),
                    prefs: &common.prefs,
                    control_on: view.control_on,
                    controlled_by: holder.as_deref(),
                    fullscreen: self.fullscreen,
                    stats_on: common.stats_on,
                    low_latency_changed: common.low_latency_changed,
                };
                actions = screen_toolbar_ui::show(ui, &input);
                self.toolbar_height = ui.min_rect().height();
            });
            if view.is_over() || !status.is_empty() {
                egui::TopBottomPanel::top("screen_status").show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(&status);
                        if view.is_over() {
                            if ui.button("Erneut verbinden").clicked() {
                                view.reconnect(common, display);
                            }
                            if ui.button("Schließen").clicked() {
                                out.push(Transition::Close);
                            }
                        }
                    });
                });
            }
        }
        let mut result = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let Some(session) = &view.session else {
                    ui.centered_and_justified(|ui| ui.label(&status));
                    return;
                };
                let (state, has_end) = {
                    let info = session.info();
                    (info.state, info.end.is_some())
                };
                let stats = common
                    .stats_on
                    .then(|| stats_lines(&session.info(), view.widget.unmapped_keys()));
                let params = ViewParams {
                    fit: common.prefs.fit,
                    control: view.control_on && !has_end,
                    dimmed: state != SessionState::Streaming,
                    status: &status,
                    stats: stats.as_deref(),
                };
                result = Some(view.widget.show(ui, session, &params));
            });
        if let Some(result) = result {
            view.follow_size(result.display, &common.prefs);
            for local in result.locals {
                match local {
                    LocalKey::Fullscreen => out.push(Transition::Fullscreen),
                    LocalKey::Stats => common.stats_on = !common.stats_on,
                    LocalKey::ToggleControl => {
                        let offered = view.session.as_ref().is_some_and(|s| {
                            s.info()
                                .hello
                                .as_ref()
                                .is_some_and(screen_toolbar_ui::control_offered)
                        });
                        if offered {
                            actions.push(ToolbarAction::Control(!view.control_on));
                        }
                    }
                }
            }
            if result.escape && self.fullscreen && !view.control_on {
                out.push(Transition::Fullscreen);
            }
        }
        for action in actions {
            match &action {
                ToolbarAction::Device(target) => out.push(Transition::View(target.clone())),
                ToolbarAction::AllDevices => out.push(Transition::Grid),
                ToolbarAction::Fullscreen => out.push(Transition::Fullscreen),
                ToolbarAction::Fit(fit) => self.change_prefs(|prefs| prefs.fit = *fit),
                ToolbarAction::Sharpness(on) => self.change_prefs(|prefs| prefs.sharpness = *on),
                ToolbarAction::Lossless(on) => self.change_prefs(|prefs| prefs.lossless = *on),
                ToolbarAction::LowLatency(on) => {
                    self.change_prefs(|prefs| prefs.low_latency = *on);
                    self.common.low_latency_changed = true;
                }
                ToolbarAction::Stats(on) => self.common.stats_on = *on,
                _ => {}
            }
            if let Mode::View(view) = &mut self.mode {
                view.apply(&action, &mut self.common, display);
            }
        }
        out
    }

    /// Changes a setting and stores it.
    fn change_prefs(&mut self, change: impl FnOnce(&mut ViewerPrefs)) {
        change(&mut self.common.prefs);
        let _ = self.common.prefs.save();
    }

    fn transition(&mut self, ctx: &egui::Context, transition: Transition) {
        match transition {
            Transition::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
            Transition::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                self.toolbar_open = false;
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
            }
            Transition::Grid => self.to_grid(ctx),
            Transition::View(target) => self.to_view(ctx, target),
        }
    }

    fn to_grid(&mut self, ctx: &egui::Context) {
        let backend = self.common.backend.clone();
        let mut grid =
            ScreenTileGrid::new(self.scope.clone(), backend, self.common.devices.clone());
        if let Mode::View(view) = &mut self.mode {
            if let Some(session) = view.session.take() {
                view.widget.release_input(&session);
                let edge = preview_edge(240.0, ctx.pixels_per_point());
                if let Some(session) = to_tile(session, edge, DEFAULT_PREVIEW_INTERVAL_MS) {
                    grid.adopt(session);
                }
            }
        }
        self.mode = Mode::Grid(Box::new(grid));
    }

    fn to_view(&mut self, ctx: &egui::Context, target: ScreenTarget) {
        let display = Self::display_size(ctx);
        let tile = match &mut self.mode {
            Mode::Grid(grid) => grid.take_session(&target),
            Mode::View(_) => None,
        };
        let open = ViewState::open_options(&self.common, display);
        let state = match tile {
            Some(session) => {
                let session = to_view(&self.common.backend, Some(session), &target, &open);
                ViewState::with_session(target, session, display, self.common.gpu)
            }
            None => ViewState::new(target, self.common.gpu),
        };
        self.mode = Mode::View(Box::new(state));
    }
}

impl eframe::App for ScreenViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if crate::app::update_stop_ui::close_requested(ctx) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if self.want_fullscreen {
            self.want_fullscreen = false;
            self.fullscreen = true;
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
        }
        self.set_title(ctx);
        let transitions = match self.mode {
            Mode::View(_) => self.show_view(ctx),
            Mode::Grid(_) => self.show_grid(ctx),
        };
        for transition in transitions {
            self.transition(ctx, transition);
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}
