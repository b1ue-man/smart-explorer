//! One viewer transport loop with its original in-flight and image owners.
use super::commands::command_allowed;
use super::{adopt, invalid, refresh, viewer_images, Command, Shared, ViewerFrame};
use crate::daemon::{LoginCommand, LoginViewer};
use crate::screen::core::input_model::{InputEvent, TouchPhase};
use std::{
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

pub(super) fn run(
    shared: &Shared,
    mut viewer: LoginViewer,
    input: mpsc::Receiver<Command>,
) -> Result<(), String> {
    if shared.stopped.load(Ordering::SeqCst) {
        viewer.close();
        return Ok(());
    }
    if shared
        .state
        .lock()
        .map_err(|_| "login viewer state is unavailable")?
        .visible
        .elapsed()
        > Duration::from_secs(2)
    {
        viewer.close();
        return Err("login viewer owner disappeared during connection setup".into());
    }
    let started = exchange(shared, &mut viewer, LoginCommand::Start)?;
    require_state(&started, "ready")?;
    adopt(shared, &viewer)?;
    let mut frame_id = 1u64;
    let mut captured = Instant::now() - Duration::from_secs(1);
    let mut had_input = false;
    loop {
        if shared.stopped.load(Ordering::SeqCst) {
            viewer.close();
            return Ok(());
        }
        if shared
            .state
            .lock()
            .map_err(|_| "login viewer state is unavailable")?
            .visible
            .elapsed()
            > Duration::from_secs(2)
        {
            viewer.close();
            return Err("login viewer is no longer visibly owned".into());
        }
        if shared.release.swap(false, Ordering::SeqCst) && had_input {
            exchange(shared, &mut viewer, LoginCommand::ReleaseInputs)?;
            had_input = false;
            if let Ok(mut state) = shared.state.lock() {
                state.pressed.release_all();
            }
            adopt(shared, &viewer)?;
        }
        if let Some(end) = viewer.poll_end()? {
            return Err(end.detail.unwrap_or(end.code));
        }
        match input.recv_timeout(Duration::from_millis(10)) {
            Ok(command) => {
                let state = shared
                    .state
                    .lock()
                    .map_err(|_| "login viewer state is unavailable")?;
                let allowed = command_allowed(&state, &command);
                drop(state);
                if !allowed {
                    if let Ok(mut state) = shared.state.lock() {
                        state.snapshot.notice = Some("Eingabe wegen geänderter Bild-, Fokus- oder Rechtebindung verworfen; keine automatische Wiederholung.".into());
                    }
                    shared.release.store(true, Ordering::SeqCst);
                    continue;
                }
                match command {
                    Command::Input(_, _, events) => {
                        had_input = true;
                        if let Ok(mut state) = shared.state.lock() {
                            for event in &events {
                                if !matches!(
                                    event,
                                    InputEvent::Key { down: false, .. }
                                        | InputEvent::Button { down: false, .. }
                                        | InputEvent::Touch {
                                            phase: TouchPhase::End | TouchPhase::Cancel,
                                            ..
                                        }
                                ) {
                                    state.pressed.observe(event);
                                }
                            }
                        }
                        let attempted = events.clone();
                        let result = exchange(shared, &mut viewer, LoginCommand::Input { events })?;
                        require_state(&result, "input_applied")?;
                        if let Ok(mut state) = shared.state.lock() {
                            for event in &attempted {
                                state.pressed.observe(event);
                            }
                        }
                    }
                    Command::Sas(_, _) => {
                        let result = exchange(shared, &mut viewer, LoginCommand::SecureAttention)?;
                        let text = result.reply.packet.as_ref().map(|packet| {
                            match packet.reply.reason.as_deref() {
                                Some(reason) => format!(
                                    "Sichere Aufmerksamkeit: {} · {reason}",
                                    packet.reply.state
                                ),
                                None => format!("Sichere Aufmerksamkeit: {}", packet.reply.state),
                            }
                        });
                        if let Ok(mut state) = shared.state.lock() {
                            state.snapshot.notice = text;
                        }
                    }
                }
                adopt(shared, &viewer)?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                viewer.close();
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if shared.release.load(Ordering::SeqCst) {
            continue;
        }
        if captured.elapsed() >= Duration::from_millis(100) {
            if !viewer_images::can_capture(&shared.images).map_err(invalid)? {
                // Latest has not left this owner and may be superseded. Images
                // already transferred to UI/JNI continue consuming their slots.
                if let Ok(mut state) = shared.state.lock() {
                    state.frame.take();
                }
                if !viewer_images::can_capture(&shared.images).map_err(invalid)? {
                    continue;
                }
            }
            let frame = exchange(shared, &mut viewer, LoginCommand::Capture { frame_id })?;
            adopt(shared, &viewer)?;
            if let Some(image) = frame.image {
                let packet = frame
                    .reply
                    .packet
                    .as_ref()
                    .ok_or_else(|| "login capture has no bounded metadata".to_string())?;
                let mut state = shared
                    .state
                    .lock()
                    .map_err(|_| "login viewer state is unavailable")?;
                if shared.stopped.load(Ordering::SeqCst) {
                    state.guard.close();
                    state.frame = None;
                    return Ok(());
                }
                let token = state
                    .guard
                    .frame(frame_id, packet.reply.width, packet.reply.height)
                    .map_err(invalid)?;
                state.frame = Some(ViewerFrame { token, image });
                refresh(&mut state);
                shared.changed.notify_all();
            } else if !matches!(
                frame.reply.view.state.as_str(),
                "unchanged" | "ready" | "pixels"
            ) {
                return Err(frame
                    .reply
                    .packet
                    .as_ref()
                    .and_then(|packet| packet.reply.reason.clone())
                    .unwrap_or_else(|| {
                        format!("login capture ended: {}", frame.reply.view.state)
                    }));
            }
            frame_id = frame_id
                .checked_add(1)
                .filter(|id| *id <= i64::MAX as u64)
                .ok_or_else(|| "login frame sequence exhausted".to_string())?;
            captured = Instant::now();
        }
    }
}
pub(super) fn begin_exchange(shared: &Shared, command: &LoginCommand) {
    shared.operating.store(true, Ordering::SeqCst);
    shared.input_flight.store(
        matches!(
            command,
            LoginCommand::Input { .. } | LoginCommand::SecureAttention
        ),
        Ordering::SeqCst,
    );
}
fn exchange(
    shared: &Shared,
    viewer: &mut LoginViewer,
    command: LoginCommand,
) -> Result<crate::daemon::LoginFrame, String> {
    struct InFlight<'a>(&'a Shared);
    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            self.0.operating.store(false, Ordering::SeqCst);
            self.0.input_flight.store(false, Ordering::SeqCst);
        }
    }
    begin_exchange(shared, &command);
    let _flight = InFlight(shared);
    if shared.stopped.load(Ordering::SeqCst) {
        viewer.close();
        return Err("login viewer owner already ended".into());
    }
    let has_pressed = !shared
        .state
        .lock()
        .map_err(|_| "login viewer state is unavailable")?
        .pressed
        .is_empty();
    if !matches!(command, LoginCommand::ReleaseInputs)
        && shared.release.load(Ordering::SeqCst)
        && (shared.input_flight.load(Ordering::SeqCst) || has_pressed)
    {
        viewer.close();
        return Err(
            "login display/focus changed before an operation was sent; owner closed without retry"
                .into(),
        );
    }
    viewer.command(command)
}
fn require_state(frame: &crate::daemon::LoginFrame, expected: &str) -> Result<(), String> {
    let reply = frame
        .reply
        .packet
        .as_ref()
        .ok_or_else(|| "login operation has no broker reply".to_string())?;
    if reply.reply.state != expected {
        return Err(format!(
            "{}: {}",
            reply.reply.state,
            reply
                .reply
                .reason
                .as_deref()
                .unwrap_or("login operation was not admitted")
        ));
    }
    Ok(())
}
