//! Family events share the decode thread, preserving control-event ordering.
use std::io;
use std::sync::Arc;
use super::screen_session::Shared;
use crate::screen::core::wire_control::{HostFrame, ViewerFrame};

pub(crate) fn receive(shared:&Shared, send:&Arc<dyn Fn(&ViewerFrame)->io::Result<()>+Send+Sync>, frame:&HostFrame)
    ->Option<io::Result<bool>> {
    let layout_changed={
        let info=shared.info();
        let incoming=match frame {HostFrame::FamilySnapshot(snapshot)=>Some(snapshot),
            HostFrame::FamilyOperation(result)=>result.snapshot.as_ref(),_=>None};
        info.family.as_ref().and_then(|family|family.snapshot.as_ref()).zip(incoming)
            .is_some_and(|(old,new)|new.revision>=old.revision&&new.layout_revision!=old.layout_revision)
    };
    // Close the rendered floor before publishing geometry to the GUI thread.
    if layout_changed {shared.cancel_presentation();}
    let mut info=shared.info();
    if let HostFrame::IconReply(response)=frame {
        let expected=info.family.as_ref().and_then(|family|family.snapshot.as_ref())
            .and_then(|snapshot|snapshot.members.iter().find(|m|m.identity==snapshot.root))
            .and_then(|m|m.icon.as_ref());
        if let Some(expected)=expected.filter(|icon|**icon==response.request.icon) {
            if let Some(icon)=response.content.as_ref().and_then(|content|content.bytes_for(expected).ok())
                .and_then(|png|eframe::icon_data::from_png_bytes(&png).ok()) {
                info.family_icon=Some(Arc::new(icon));
            }
        }
        return None;
    }
    let previous_icon=info.family.as_ref().and_then(|family|family.snapshot.as_ref())
        .and_then(|snapshot|snapshot.members.iter().find(|m|m.identity==snapshot.root)).and_then(|m|m.icon.clone());
    let family=info.family.as_mut()?;
    let result=match frame {
        HostFrame::FamilySnapshot(snapshot)=>family.receive(snapshot.clone()).map(|changed|(changed,Vec::new())),
        HostFrame::FamilyOperation(result)=>family.result(result.clone()),
        HostFrame::FamilyInput(result)=>{
            if result.reason.is_some()||result.blocked.is_some() {
                family.disarm();family.notice=Some(result.reason.clone().unwrap_or_else(||format!("Eingabe gesperrt: {:?}",result.blocked)));
            }
            Ok((false,Vec::new()))
        }
        HostFrame::ControlState(state) if state.blocked.is_some()||state.holder.is_none()=>{
            family.disarm();return None;
        }
        HostFrame::State(report) if report.state != crate::screen::core::session_state::SessionState::Streaming=>{
            family.disarm();return None;
        }
        _=>return None,
    };
    let current_icon=family.snapshot.as_ref().and_then(|snapshot|snapshot.members.iter()
        .find(|m|m.identity==snapshot.root)).and_then(|m|m.icon.clone());
    if previous_icon!=current_icon {info.family_icon=None;}
    drop(info);
    Some(result.and_then(|(changed,frames)|{
        // A pending first click may only leave after the actual rendered floor
        // remains open. Layout changes discard that queued input in the gate.
        let ready=shared.resume.lock().unwrap_or_else(std::sync::PoisonError::into_inner).ready();
        if ready {
            for frame in frames {send(&frame)?;}
        }
        Ok(changed)
    }))
}
