//! Desktop wiring around the shared family gate; no renderer replacement.
use super::screen_session::ViewerSession;
use crate::remote_window::{WindowIdentity, WindowOperation};
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::wire_control::ViewerFrame;

pub(crate) fn input(session: &ViewerSession, events: Vec<InputEvent>) {
    if !session.input_ready() {
        return;
    }
    let frames = {
        let mut info = session.info();
        let Some(family) = &mut info.family else {
            return;
        };
        match family.input(events) {
            Ok(frames) => frames,
            Err(error) => {
                family.notice = Some(error.to_string());
                return;
            }
        }
    };
    if !session.input_ready() {
        disarm(session);
        return;
    }
    for frame in frames {
        session.send(frame);
    }
}

pub(crate) fn operate(
    session: &ViewerSession,
    identity: WindowIdentity,
    operation: WindowOperation,
) {
    let request = {
        let mut info = session.info();
        let Some(family) = &mut info.family else {
            return;
        };
        match family.operation(identity, operation) {
            Ok(request) => request,
            Err(error) => {
                family.notice = Some(error.to_string());
                return;
            }
        }
    };
    session.send(ViewerFrame::FamilyOperation(request));
}

pub(crate) fn disarm(session: &ViewerSession) {
    if let Some(family) = &mut session.info().family {
        family.disarm();
    }
}
