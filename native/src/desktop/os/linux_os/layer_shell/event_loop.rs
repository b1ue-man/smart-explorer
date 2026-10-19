//! The loop of the layer process (DSK-M7, U37-8, U37-9): one `calloop` loop
//! with the Wayland queue and a ping that the channel's reader thread and
//! the egui contexts use to wake it. After every wake-up it takes the shell's
//! messages, the repaint requests, the heartbeat, the key repeat and the
//! frames that are due. A frame is drawn only after the compositor's frame
//! callback of the previous one, and only when something asked for it.
//! The update stop (75) and the end of the channel (76) end the process in
//! their own threads, as with eframe; losing the compositor ends it with 76.
use std::time::{Duration, Instant};

use calloop::ping::PingSource;
use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;
use wayland_client::EventQueue;

use super::render::Drawn;
use super::state::LayerState;
use crate::app::desktop_surface_view_model::ALIVE_EVERY;
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage, EXIT_CHANNEL_END};

/// A frame that could not be shown is tried again after this.
const RETRY_AFTER: Duration = Duration::from_millis(50);

/// Runs the loop until the process has to end: its exit code.
pub(super) fn run(mut state: LayerState, queue: EventQueue<LayerState>, ping: PingSource) -> i32 {
    let mut event_loop: EventLoop<LayerState> = match EventLoop::try_new() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("se Desktop: Layer-Backend: Ereignisschleife: {error}");
            return 1;
        }
    };
    let handle = event_loop.handle();
    if let Err(error) = WaylandSource::new(state.conn.clone(), queue).insert(handle.clone()) {
        eprintln!("se Desktop: Layer-Backend: Wayland-Quelle: {error}");
        return 1;
    }
    if let Err(error) = handle.insert_source(ping, |_, _, _| {}) {
        eprintln!("se Desktop: Layer-Backend: Weckruf: {error}");
        return 1;
    }
    loop {
        state.after_dispatch(Instant::now());
        if let Some(code) = state.exit {
            return code;
        }
        let timeout = state.next_deadline(Instant::now());
        if let Err(error) = event_loop.dispatch(timeout, &mut state) {
            // The compositor went away: the shell starts the process anew
            // without counting a crash.
            eprintln!("se Desktop: Layer-Backend: Verbindung verloren: {error}");
            return EXIT_CHANNEL_END;
        }
    }
}

impl LayerState {
    /// Everything that follows a wake-up.
    pub(super) fn after_dispatch(&mut self, now: Instant) {
        self.process_inbox();
        self.process_repaints(now);
        self.heartbeat(now);
        self.tick_repeat(now);
        if std::mem::take(&mut self.outputs_changed) {
            // The shell reads the outputs again and restarts the surfaces.
            self.shared.sender.send(SurfaceMessage::MonitorsChanged);
        }
        for index in 0..self.views.len() {
            if self.views[index].wants_draw(now) {
                self.draw_view(index, now);
            }
        }
    }

    /// How long the loop may sleep: until the next heartbeat, key repeat or
    /// requested frame of a view that is allowed to draw.
    pub(super) fn next_deadline(&self, now: Instant) -> Option<Duration> {
        let mut next: Option<Instant> = None;
        let mut times: Vec<Instant> = Vec::new();
        if self.shared.send_alive {
            times.push(self.shared.next_alive);
        }
        times.extend(
            self.views
                .iter()
                .filter(|view| !view.frame.frame_pending)
                .filter_map(|view| view.frame.redraw_at),
        );
        times.extend(self.input.next_repeat());
        for time in times {
            next = Some(next.map_or(time, |old| old.min(time)));
        }
        next.map(|time| time.saturating_duration_since(now))
    }

    /// The shell's messages: a model goes to the view of its output (no key:
    /// the first), the configuration to every view.
    fn process_inbox(&mut self) {
        while let Ok(message) = self.shared.inbox.try_recv() {
            match message {
                ShellMessage::Model {
                    rev,
                    monitor,
                    model,
                } => {
                    let target = match &monitor {
                        Some(key) => self.views.iter_mut().find(|view| &view.info.key == key),
                        None => self.views.first_mut(),
                    };
                    if let Some(view) = target {
                        let model = ShellMessage::Model {
                            rev,
                            monitor: None,
                            model,
                        };
                        if view.view.absorb(model) {
                            view.frame.dirty = true;
                        }
                    }
                }
                config @ ShellMessage::Config { .. } => {
                    for view in &mut self.views {
                        if view.view.absorb(config.clone()) {
                            view.frame.dirty = true;
                        }
                    }
                }
                // The reader thread ends the process on `stop`.
                ShellMessage::Hello { .. } | ShellMessage::Stop { .. } => {}
            }
        }
    }

    /// The repaint requests of the egui contexts since the last turn.
    fn process_repaints(&mut self, now: Instant) {
        let requests = match self.shared.repaints.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => return,
        };
        for (id, delay) in requests {
            let Some(view) = self.views.iter_mut().find(|view| view.id == id) else {
                continue;
            };
            if delay.is_zero() {
                view.frame.dirty = true;
            } else {
                let at = now + delay;
                view.frame.redraw_at = Some(view.frame.redraw_at.map_or(at, |old| old.min(at)));
            }
        }
    }

    /// `alive` once a second (C5.5), independent of any frame.
    fn heartbeat(&mut self, now: Instant) {
        if self.shared.send_alive && now >= self.shared.next_alive {
            self.shared.sender.send(SurfaceMessage::Alive);
            self.shared.next_alive = now + ALIVE_EVERY;
        }
    }

    /// One frame of the view at `index`: the egui pass, the cursor and the
    /// copied text, then the paint and the frame callback.
    fn draw_view(&mut self, index: usize, now: Instant) {
        let Self {
            views,
            gpu,
            input,
            clipboard,
            conn,
            qh,
            started,
            exit,
            ..
        } = self;
        let Some(gpu) = gpu.as_ref() else {
            return;
        };
        let view = &mut views[index];
        view.frame.dirty = false;
        view.frame.redraw_at = None;
        let raw = view.raw_input(*started, input.modifiers, gpu.max_texture_side());
        let mut output = view.run_ui(raw);
        let cursor = output.platform_output.cursor_icon;
        if view.frame.pointer_inside && input.cursor != Some(cursor) {
            input.apply_cursor(conn, cursor);
        }
        let copied = std::mem::take(&mut output.platform_output.copied_text);
        if let (false, Some(clipboard)) = (copied.is_empty(), clipboard.as_ref()) {
            clipboard.write_text(&copied);
        }
        let Some(render) = view.render.as_mut() else {
            return;
        };
        let window = &view.window;
        let drawn = render.draw(gpu, &view.ctx, output, || {
            window.apply_scale();
            window.surface.frame(qh, window.surface.clone());
        });
        match drawn {
            Drawn::Presented => view.frame.frame_pending = true,
            Drawn::Retry | Drawn::Skipped => view.frame.redraw_at = Some(now + RETRY_AFTER),
            Drawn::OutOfMemory => {
                eprintln!("se Desktop: Layer-Backend: Grafikspeicher erschöpft");
                exit.get_or_insert(1);
            }
        }
    }
}
