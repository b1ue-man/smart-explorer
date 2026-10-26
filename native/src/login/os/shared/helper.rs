//! Dedicated capture/input process. No network endpoint or codec is created.
use std::{io, sync::mpsc, time::{Duration, Instant}};
use crate::login::core::{model::{Operation, MAX_PIXELS}, wire::{self, HelperCall, HelperCommand, HelperInit, Reply, PIXELS}};
use crate::screen::core::{host_backend::SourceFrame, input_model::{PressedInputs, InputEvent, TouchPhase}};
use super::{denied, super::platform};

pub(crate) fn run(probe: bool) -> io::Result<()> {
    let (mut input, mut output) = platform::helper_channel()?;
    let init: HelperInit = wire::receive(&mut input)?;
    if init.cookie.len() != 64 { return Err(denied("invalid inherited login helper capability")); }
    let actual = platform::prepare_helper(&init.binding, init.parent_id, probe)?;
    if probe { return platform::run_probe(input, output, init, actual); }
    if actual != init.binding { return Err(denied("login desktop changed before helper initialization")); }
    let mut source = crate::screen::os::open_login_source()?;
    if !context_matches(&actual)? { return Err(denied("login desktop geometry changed during initialization")); }
    let mut ready = Reply::state("ready"); ready.binding = Some(actual.clone());
    wire::send(&mut output, &ready)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new().name("login-helper-input".into()).spawn(move || loop {
        let call = wire::receive::<HelperCall>(&mut input);
        let end = call.is_err(); if sender.send(call).is_err() || end { break; }
    })?;
    let mut pressed = PressedInputs::default();
    let mut last_request = Instant::now();
    let result = (|| loop {
        // A dedicated fresh helper replaces the old context; no UI thread is moved.
        if !context_matches(&actual)? || last_request.elapsed() > Duration::from_secs(5) {
            return Err(denied("login desktop, capture geometry or helper lease ended"));
        }
        let call = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(call) => call?, Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return Ok(()),
        };
        last_request = Instant::now();
        if call.cookie != init.cookie { return Err(denied("wrong inherited login helper capability")); }
        let HelperCommand::Operation(operation) = call.command else { return Err(denied("capture helper does not accept metadata-probe commands")); };
        operation.validate().map_err(denied)?;
        if !context_matches(&actual)? { return Err(denied("login desktop or capture geometry changed before operation")); }
        match operation {
            Operation::Capture { frame_id } => match source.capture(frame_id, false)? {
                SourceFrame::Frame(image) => {
                    let size = image.size(); let bytes = image.data();
                    if bytes.len() > MAX_PIXELS || bytes.len() != size.width as usize * size.height as usize * 4 {
                        return Err(denied("login capture exceeds bounded raw frame"));
                    }
                    // Recheck after capture so a context switch cannot publish stale pixels.
                    if !context_matches(&actual)? { return Err(denied("login desktop or capture geometry changed during capture")); }
                    let mut reply = Reply::state("pixels"); reply.width = size.width; reply.height = size.height; reply.pixel_bytes = bytes.len();
                    wire::send(&mut output, &reply)?; wire::write_frame(&mut output, PIXELS, bytes)?;
                }
                SourceFrame::Unchanged => wire::send(&mut output, &Reply::state("unchanged"))?,
                SourceFrame::Paused(_) => wire::send(&mut output, &Reply::state("paused"))?,
            },
            Operation::Input { events } => {
                // Retain attempted downs too: partial OS injection still needs releases.
                for event in &events {
                    if !matches!(event, InputEvent::Key { down: false, .. } | InputEvent::Button { down: false, .. }
                        | InputEvent::Touch { phase: TouchPhase::End | TouchPhase::Cancel, .. }) { pressed.observe(event); }
                }
                crate::screen::os::inject_login_input(&events)?;
                for event in &events { pressed.observe(event); }
                wire::send(&mut output, &Reply::state("input_applied"))?;
            }
            Operation::ReleaseInputs => {
                let releases = pressed.clone().release_all();
                crate::screen::os::inject_login_input(&releases)?;
                pressed.release_all();
                wire::send(&mut output, &Reply::state("inputs_released"))?;
            }
            Operation::Stop => return Ok(()),
            Operation::Start | Operation::SecureAttention => return Err(denied("operation belongs to the broker")),
        }
    })();
    // The OS adapter keeps the old bound desktop for releases even after a switch.
    let released = crate::screen::os::inject_login_input(&pressed.release_all());
    platform::finish_helper();
    result.and(released)
}

fn context_matches(binding: &crate::login::core::model::SessionBinding) -> io::Result<bool> {
    Ok(platform::helper_binding_matches(binding)? && crate::screen::os::login_geometry_current()?)
}
