use eframe::egui;
use std::sync::Arc;
use crate::rdp::{AuthenticatedProbe,CertificatePolicy,StartRequest};
use crate::rdp::core::HostKind;
use super::State;

pub(super) fn show(ui:&mut egui::Ui,id:egui::Id,probe:&Arc<AuthenticatedProbe>,state:&mut State)->Option<(Arc<AuthenticatedProbe>,StartRequest)> {
    let report=probe.report();
    if let Some(reason)=&report.unavailable_reason {ui.label(reason);}
    for diagnostic in &report.diagnostics {ui.label(diagnostic);}
    if report.services.is_empty() {return None;}
    let before=(state.service.clone(),state.client.clone(),state.app.clone(),state.lifetime.clone());
    egui::ComboBox::from_id_salt(id.with("service")).selected_text(&state.service).show_ui(ui,|ui| {
        for service in &report.services {ui.selectable_value(&mut state.service,service.id.clone(),format!("{} · {}",service.id,kind(service.kind)));}
    });
    let service=report.services.iter().find(|s|s.id==state.service)?;
    ui.label(service.session_mode.notice());
    ui.label(format!("Dienst: {} · Version: {} · tatsächlicher Listenerport: {}",service.id,
        service.version.as_deref().unwrap_or("nicht ermittelt"),service.port.map_or_else(||"nicht belegt".into(),|p|p.to_string())));
    ui.label(format!("TLS: {} · NLA: {}",fact(service.tls),fact(service.nla)));
    if let Some(certificate)=&service.certificate_sha256 {
        ui.label("SHA-256 des RDP-Zertifikats aus dem authentifizierten Hostbestand:");
        ui.monospace(certificate.as_str());
        if ui.button("Zertifikatfingerprint kopieren").clicked() {ui.ctx().copy_text(certificate.as_str().into());}
    } else {ui.colored_label(egui::Color32::RED,"Der Host hat keinen belegten SHA-256-Zertifikatfingerprint geliefert.");}
    if let Some(source)=&service.certificate_source {ui.label(source);}
    if let Some(reason)=&service.unavailable_reason {ui.colored_label(egui::Color32::RED,reason);}
    egui::CollapsingHeader::new("Hostbelege").id_salt(id.with("evidence")).show(ui,|ui| {
        for evidence in &service.evidence {ui.label(evidence);}
    });
    ui.label("Vorhandener lokaler RDP-Client");
    if state.clients.is_empty() {ui.label("Kein vorhandener lokaler Client mit belegtem Zertifikatsprüfadapter. Es wird kein Client installiert.");}
    egui::ComboBox::from_id_salt(id.with("client")).selected_text(if state.client.is_empty(){"Kein Client"}else{&state.client}).show_ui(ui,|ui| {
        for client in &state.clients {ui.selectable_value(&mut state.client,client.id.clone(),format!("{}{}",client.id,if client.available{""}else{" · nicht verfügbar"}));}
    });
    let client=state.clients.iter().find(|client|client.id==state.client).cloned();
    if let Some(client)=&client {ui.label(&client.detail);if let Some(version)=&client.version {ui.label(version);}}
    ui.label("Darstellung");
    egui::ComboBox::from_id_salt(id.with("app")).selected_text(state.app.as_deref().unwrap_or("Ganzer Desktop")).show_ui(ui,|ui| {
        ui.selectable_value(&mut state.app,None,"Ganzer Desktop");
        for app in &service.remote_apps {ui.selectable_value(&mut state.app,Some(app.alias.clone()),format!("{} · {} · {}",app.display_name,app.alias,app.collection));}
    });
    if service.remote_apps.is_empty() {ui.label("Dieser Host hat kein belegtes Windows-Server/RDS-RemoteAppangebot veröffentlicht.");}
    if state.app.as_ref().is_some_and(|alias|!service.remote_apps.iter().any(|app|app.alias==*alias)) {state.app=None;}
    ui.horizontal(|ui| {ui.label("Zugang endet spätestens nach Sekunden (1–86400):");ui.text_edit_singleline(&mut state.lifetime);});
    if before!=(state.service.clone(),state.client.clone(),state.app.clone(),state.lifetime.clone()) {state.reset_consent();}
    ui.label(crate::share::tcp_forward::TCP_LOCAL_ACCESS_NOTICE);
    ui.checkbox(&mut state.local_ack,"Ich bestätige die Erreichbarkeit durch jeden lokalen Prozess.");
    ui.checkbox(&mut state.session_ack,"Ich bestätige die oben beschriebene Wirkung auf die Sitzung des Zielgeräts.");
    if client.as_ref().is_some_and(|client|client.certificate_policy==CertificatePolicy::NativeDialog) {
        ui.checkbox(&mut state.certificate_ack,"Ich vergleiche im nativen Zertifikatdialog den oben gezeigten SHA-256-Fingerprint, bevor ich die Verbindung bestätige.");
    } else if client.is_some() {ui.label("Prüfpfad dieses Adapters: Peerpin oder das ausdrücklich beschriebene native Zertifikatsvertrauen. Eine ausschließliche Pinbindung wird nicht behauptet.");}
    else {ui.label("Kein lokaler Client mit einem belegten Zertifikatsprüfpfad verfügbar.");}
    let request=(|| {
        let client=client.as_ref().ok_or_else(||"Kein lokaler RDP-Client verfügbar.".to_string())?;
        if !client.available {return Err(client.detail.clone());}
        if state.app.is_some()&&!client.remote_app {return Err("Dieser Client hat keinen belegten RemoteAppadapter.".into());}
        let request=StartRequest {service_id:service.id.clone(),client_id:client.id.clone(),certificate_policy:client.certificate_policy,
            remote_app:state.app.clone(),lifetime_secs:state.lifetime.parse().map_err(|_|"Lebensdauer muss 1–86400 Sekunden betragen.".to_string())?,
            acknowledge_local_access:state.local_ack,acknowledge_session_effect:state.session_ack,acknowledge_certificate_dialog:state.certificate_ack};
        request.validate(probe)?;Ok::<_,String>(request)
    })();
    if let Err(reason)=&request {ui.label(reason);}
    if ui.add_enabled(request.is_ok(),egui::Button::new(if state.app.is_some(){"Veröffentlichte RemoteApp öffnen"}else{"Remote Desktop öffnen"})).clicked() {
        return request.ok().map(|request|(probe.clone(),request));
    }
    None
}
fn fact(value:Option<bool>)->&'static str {match value {Some(true)=>"aktiv / belegt",Some(false)=>"aus",None=>"nicht ermittelt"}}
fn kind(value:HostKind)->&'static str {match value {HostKind::WindowsWorkstation=>"Windows Pro/Enterprise",HostKind::WindowsServer=>"Windows Server / RDS",HostKind::Gnome=>"GNOME Remote Desktop",HostKind::Krdp=>"KRdp",HostKind::Xrdp=>"xrdp"}}
