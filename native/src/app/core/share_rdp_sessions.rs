use std::sync::{Arc,Mutex,PoisonError};
use eframe::egui;

pub(super) fn show(ui:&mut egui::Ui,nodes:&[&str]) {
    ui.label("Native RDP-Sitzungen dieses Geräts");
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    let id=ui.id().with("rdp-close-result");
    let result:Arc<Mutex<Option<Result<String,String>>>>=ui.ctx().data_mut(|d|
        d.get_temp_mut_or_insert_with(id,||Arc::new(Mutex::new(None))).clone());
    if let Some(value)=&*result.lock().unwrap_or_else(PoisonError::into_inner) {
        match value {Ok(message)=>{ui.label(message);},Err(message)=>{ui.colored_label(egui::Color32::RED,message);}}
    }
    let sessions:Vec<_>=crate::rdp::owned_sessions().into_iter().filter(|s|nodes.contains(&s.tunnel.remote_node.as_str())).collect();
    if sessions.is_empty() {ui.label("Keine von diesem Smart-Explorer-Prozess gehaltene RDP-Sitzung zu diesem Gerät.");}
    for session in sessions {egui::Frame::group(ui.style()).show(ui,|ui| {
        ui.label(format!("{} · Client {} · Prozess {}",session.tunnel.id.as_str(),session.client.id,session.process_id));
        ui.label(&session.session_notice);
        ui.label(format!("RemoteApp: {} · Beziehung: {} / {}",session.remote_app.as_deref().unwrap_or("ganzer Desktop"),session.tunnel.relation_kind,session.tunnel.relation_id));
        ui.label(format!("Lokaler Listener: {} · spätester Ablauf in {} s · {} Verbindungen",session.tunnel.listener.map_or_else(||"nicht vorhanden".into(),|a|a.to_string()),
            session.tunnel.expires_at.saturating_sub(crate::share::core_now_secs()).max(0),session.tunnel.active_connections));
        if session.tunnel.closing {ui.label("Client und TCP-Lease werden geschlossen …");}
        if let Some(error)=&session.tunnel.last_error {ui.colored_label(egui::Color32::RED,error);}
        if ui.add_enabled(!session.tunnel.closing||session.tunnel.last_error.is_some(),egui::Button::new("RDP-Sitzung schließen")).clicked() {
            let (id,state,context)=(session.tunnel.id.clone(),result.clone(),ui.ctx().clone());
            let retry=state.clone();
            if let Err(error)=std::thread::Builder::new().name("device-rdp-close".into()).spawn(move || {
                let closed=crate::rdp::close_owned_session(&id).map(|found|if found{"RDP-Client und TCP-Lease geschlossen.".into()}else{"Diese RDP-Sitzung ist bereits beendet.".into()});
                *state.lock().unwrap_or_else(PoisonError::into_inner)=Some(closed);context.request_repaint();
            }) {*retry.lock().unwrap_or_else(PoisonError::into_inner)=Some(Err(error.to_string()));}
        }
    });}
}
