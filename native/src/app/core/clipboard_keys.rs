//! File-clipboard keys (Ctrl+C / Ctrl+X / Ctrl+V) as they reach the explorer:
//! egui's semantic events, the Windows key poller, and the text a keyboard
//! paste brought along (on Linux it tells whether our copy is still current).
use super::prelude::*;
use super::*;

#[derive(Default)]
pub(in crate::app) struct ClipboardKeys {
    pub(in crate::app) copy: bool,
    pub(in crate::app) cut: bool,
    pub(in crate::app) paste: bool,
    paste_text: Option<String>,
}

impl ClipboardKeys {
    /// Ctrl+C / Ctrl+X / Ctrl+V do not arrive as key events: the winit
    /// backend turns them into semantic Copy/Cut/Paste events (so text
    /// widgets work). A paste event exists only when the system clipboard
    /// holds text; our copies leave their paths there for that reason.
    pub(in crate::app) fn read_events(&mut self, events: &[egui::Event]) {
        for event in events {
            match event {
                egui::Event::Copy => self.copy = true,
                egui::Event::Cut => self.cut = true,
                egui::Event::Paste(text) => {
                    self.paste = true;
                    self.paste_text = Some(text.clone());
                }
                _ => {}
            }
        }
    }

    /// Keys the Windows background poller observed (copy, cut, paste).
    pub(in crate::app) fn add_polled(&mut self, polled: [bool; 3]) {
        self.copy |= polled[0];
        self.cut |= polled[1];
        self.paste |= polled[2];
    }
}

impl App {
    pub(in crate::app) fn run_clipboard_keys(&mut self, keys: ClipboardKeys) {
        if keys.copy {
            self.clipboard_copy_files(false);
        }
        if keys.cut {
            self.clipboard_copy_files(true);
        }
        if keys.paste {
            self.clipboard_paste_with_text(keys.paste_text.as_deref());
        }
    }

    /// Our copy leaves its paths as text on the system clipboard once a frame
    /// can write it (the marker a later paste is compared with).
    pub(in crate::app) fn flush_clip_marker(&mut self, ctx: &egui::Context) {
        if let Some(text) = self
            .clip
            .as_mut()
            .and_then(|clip| clip.take_pending_marker())
        {
            ctx.copy_text(text);
        }
    }
}

pub(in crate::app) fn take_clipboard_keys(rx: Option<&Receiver<ClipKey>>) -> ([bool; 3], bool) {
    let mut actions = [false; 3];
    let Some(rx) = rx else {
        return (actions, false);
    };
    loop {
        match rx.try_recv() {
            Ok(ClipKey::Copy) => actions[0] = true,
            Ok(ClipKey::Cut) => actions[1] = true,
            Ok(ClipKey::Paste) => actions[2] = true,
            Err(crossbeam_channel::TryRecvError::Empty) => return (actions, false),
            Err(crossbeam_channel::TryRecvError::Disconnected) => return (actions, true),
        }
    }
}

#[cfg(test)]
#[path = "copy_paste_keyboard_task_tests.rs"]
mod copy_paste_keyboard_task_tests;
