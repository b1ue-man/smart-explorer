//! „Ton“ of the desktop viewer (plan 2026-11-17 P9 „Audio“): the host's
//! sound plays in the viewer, on unless the user switched it off. The
//! setting is stored with the viewer's settings (`viewer.json` key `audio`,
//! absent is on) and is the same for the toolbar, `se screen view --audio
//! on|off` and the MCP tool `screen` (action `audio`).
//!
//! A session gets the sound only from a host that answered `audio` in
//! `HELLO_OK`: after each acceptance the viewer sends `AUDIO` with the
//! setting (`follow`), and the toolbar's „Ton“ sends it again at once
//! (`switch`). A host without the answer (older version, Android, no sound
//! capture) never gets `AUDIO`, and „Ton“ is greyed out with the reason.
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui;

use super::screen_prefs::{prefs_path, ViewerPrefs};
use super::screen_session::{SessionInfo, ViewerSession};
use super::screen_toolbar_ui::ToolbarAction;
use crate::screen::core::wire_control::ViewerFrame;

/// „Ton“ of this process: unread (`None`) or the stored value.
static AUDIO: Mutex<Option<Option<bool>>> = Mutex::new(None);
/// The session (its place in the window) and acceptance `follow` last
/// answered; `forget` clears it when the window ends its session.
static FOLLOWED: Mutex<Option<(usize, u32)>> = Mutex::new(None);
/// When the last `AUDIO_REPORT` left.
static REPORTED: Mutex<Option<Instant>> = Mutex::new(None);
/// Interval of `AUDIO_REPORT`.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// The stored „Ton“ as the file holds it (`None`: never chosen).
pub fn stored() -> Option<bool> {
    let mut known = AUDIO.lock().unwrap_or_else(PoisonError::into_inner);
    *known.get_or_insert_with(|| ViewerPrefs::load_from(&prefs_path()).audio)
}

/// Whether the host's sound plays; never chosen is on.
pub fn audio_on() -> bool {
    stored().unwrap_or(true)
}

/// Stores „Ton“ at once.
pub fn set_audio(on: bool) -> io::Result<()> {
    *AUDIO.lock().unwrap_or_else(PoisonError::into_inner) = Some(Some(on));
    let mut prefs = ViewerPrefs::load_from(&prefs_path());
    prefs.audio = Some(on);
    prefs.save_to(&prefs_path())
}

/// Whether the host of this session sends its sound in a format this
/// viewer plays.
pub fn negotiated(info: &SessionInfo) -> bool {
    info.hello
        .as_ref()
        .and_then(|hello| hello.audio)
        .is_some_and(|accept| accept.playable())
}

/// The toolbar's „Ton“ switch.
pub fn toolbar_row(ui: &mut egui::Ui, info: Option<&SessionInfo>, out: &mut Vec<ToolbarAction>) {
    let available = info.is_some_and(negotiated);
    let mut on = audio_on();
    let response = ui.add_enabled(available, egui::Checkbox::new(&mut on, "Ton"));
    let response = if available {
        response.on_hover_text("Ton des Geräts in diesem Fenster abspielen")
    } else {
        response.on_disabled_hover_text(
            "Das Gerät sendet keinen Ton (ältere Version, Android oder keine Tonaufnahme möglich)",
        )
    };
    if response.changed() {
        out.push(ToolbarAction::Audio(on));
    }
}

/// „Ton“ switched in the toolbar: stored, played or muted here, and sent
/// to a host that takes it.
pub fn switch(session: Option<&ViewerSession>, on: bool) {
    super::screen_toolbar_ui::note_save(set_audio(on));
    crate::screen::os::audio::set_playing(on);
    if let Some(session) = session {
        let takes = negotiated(&session.info());
        if takes {
            session.send(ViewerFrame::Audio(on));
        }
    }
}

/// The window ended its session: the sound stops, and the next session
/// (which may live at the same place) is asked again.
pub fn forget() {
    *FOLLOWED.lock().unwrap_or_else(PoisonError::into_inner) = None;
    crate::screen::os::audio::set_playing(false);
}

/// Once per frame of the window: after each acceptance of a host that
/// sends sound, asks for it with the setting; while the sound plays, sends
/// the player's statistics to the host once a second (`AUDIO_REPORT`, for
/// `se screen sessions --json`).
pub fn follow(session: Option<&ViewerSession>) {
    let Some(session) = session else {
        return;
    };
    let (accepted, takes) = {
        let info = session.info();
        (info.accepted, negotiated(&info))
    };
    let key = (std::ptr::from_ref(session) as usize, accepted);
    let known = {
        let mut followed = FOLLOWED.lock().unwrap_or_else(PoisonError::into_inner);
        let known = accepted == 0 || *followed == Some(key);
        if !known {
            *followed = Some(key);
        }
        known
    };
    if !known {
        let on = audio_on() && takes;
        crate::screen::os::audio::set_playing(on);
        if takes {
            session.send(ViewerFrame::Audio(on));
        }
    }
    if takes {
        report_now(session);
    }
}

/// Sends the player's statistics when a second has passed since the last.
fn report_now(session: &ViewerSession) {
    let Some(report) = crate::screen::os::audio::report() else {
        return;
    };
    {
        let mut last = REPORTED.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| at.elapsed() < REPORT_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
    }
    session.send(ViewerFrame::AudioReport(report));
}

fn ms(us: u32) -> String {
    format!("{:.1}", f64::from(us) / 1_000.0).replace('.', ",")
}

/// The line of the viewer's statistics for a host that sends sound.
pub fn overlay_line(info: &SessionInfo) -> Option<String> {
    if !negotiated(info) {
        return None;
    }
    if !audio_on() {
        return Some("Ton: aus".into());
    }
    let Some(report) = crate::screen::os::audio::report() else {
        return Some("Ton: wartet auf den Host".into());
    };
    Some(format!(
        "Ton: Puffer {} ms (Ziel {} ms), Jitter {} ms, Ausgabe {} ms; verloren {}, \
         Aussetzer {}, gekürzt {}, verspätet {}",
        ms(report.depth_us),
        ms(report.target_us),
        ms(report.jitter_us),
        ms(report.output_us),
        report.lost,
        report.underruns,
        report.trimmed,
        report.late,
    ))
}
