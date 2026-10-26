//! A family is its own native window; the existing renderer owns frame proof.
use std::sync::Arc;
use std::time::{Duration,Instant};
use eframe::egui::{self,ViewportCommand};
use super::screen_prefs::ViewerPrefs;
use super::screen_session::{Backend,ViewerSession};
use super::screen_target::ScreenTarget;
use super::screen_view_widget::{ViewWidget,ViewParams};
use crate::remote_window::{FamilySnapshot,WindowOperation};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_control::{ControlAction,ControlRequest,ViewerFrame};

pub(crate) fn run(target:ScreenTarget)->i32 {
    let prefs=ViewerPrefs::load();
    let fullscreen=target.fullscreen;
    let viewport=||egui::ViewportBuilder::default().with_title("Fernfenster")
        .with_app_id("app.smartexplorer.family").with_inner_size([960.0,640.0])
        .with_min_inner_size([320.0,200.0]).with_fullscreen(fullscreen).with_icon(crate::window_icon());
    let first=target.clone();let first_prefs=prefs.clone();
    let result=eframe::run_native("app.smartexplorer.family",eframe::NativeOptions {
        renderer:eframe::Renderer::Wgpu,wgpu_options:super::screen_viewer_app::wgpu_configuration(prefs.low_latency),
        viewport:viewport(),..Default::default()
    },Box::new(move|cc|Ok(Box::new(FamilyViewerApp::new(cc,first,first_prefs)))));
    if result.is_ok(){return 0;}
    let result=eframe::run_native("app.smartexplorer.family",eframe::NativeOptions {
        renderer:eframe::Renderer::Glow,viewport:viewport(),..Default::default()
    },Box::new(move|cc|Ok(Box::new(FamilyViewerApp::new(cc,target,prefs)))));
    if let Err(error)=result {eprintln!("Fernfenster: {error}");1}else{0}
}

struct FamilyViewerApp {
    target:ScreenTarget,backend:Backend,session:Option<ViewerSession>,widget:ViewWidget,
    devices:super::screen_devices::Devices,
    prefs:ViewerPrefs,device:String,error:Option<String>,menu:bool,diagnostics:bool,
    control:bool,disconnect:bool,title:String,icon:Option<Arc<egui::IconData>>,
    last_display:Option<FrameSize>,resize_at:Option<Instant>,resize_sent:Option<FrameSize>,
    remote_resize:bool,initial_until:Instant,
    local_block:bool,fullscreen:bool,
}
impl FamilyViewerApp {
    fn new(cc:&eframe::CreationContext<'_>,target:ScreenTarget,prefs:ViewerPrefs)->Self {
        let fullscreen=target.fullscreen;
        let ctx=&cc.egui_ctx;
        ctx.data_mut(|d|d.insert_temp(egui::Id::new("screen_renderer_wgpu"),cc.wgpu_render_state.is_some()));
        let gpu=cc.wgpu_render_state.as_ref().is_some_and(super::screen_wgpu_yuv::install);
        let device=target.device_id.clone()
            .unwrap_or_else(||target.relation_id.clone());
        Self {control:target.control,target,backend:Backend::production(ctx),session:None,
            devices:super::screen_devices::Devices::start(ctx),widget:ViewWidget::new(gpu),prefs,device,error:None,menu:false,diagnostics:false,
            disconnect:false,title:String::new(),icon:None,last_display:None,resize_at:None,
            resize_sent:None,remote_resize:true,initial_until:Instant::now()+Duration::from_secs(6),local_block:false,fullscreen}
    }
    fn open(&mut self,ctx:&egui::Context) {
        if self.session.is_some()||self.error.is_some(){return;}
        {
            let state=self.devices.lock();
            let Some(profiles)=&state.profiles else {return;};
            if let Err(error)=self.target.check_profiles(profiles) {self.error=Some(error);return;}
            if let Some(name)=self.target.device_name(profiles){self.device=name;}
        }
        let extent=ctx.screen_rect().size()*ctx.pixels_per_point();
        let display=FrameSize::new(extent.x.max(1.0) as u32,extent.y.max(1.0) as u32);
        let request=super::screen_pool::full_request(&self.backend.offers,self.target.source.clone(),display,self.control);
        self.session=Some(ViewerSession::open(&self.backend,self.target.clone(),request,self.control,true,0));
    }
    fn snapshot(&self)->Option<FamilySnapshot> {
        self.session.as_ref()?.info().family.as_ref()?.snapshot.clone()
    }
    fn stop(&mut self,ctx:&egui::Context) {
        self.disconnect=true;
        if let Some(session)=&self.session {self.widget.release_input(session);session.close();}
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    fn close_program(&self) {
        if let (Some(session),Some(snapshot))=(&self.session,self.snapshot()) {
            // Keep the source/session alive: WM_CLOSE may create a save dialog.
            super::family_viewer_session::operate(session,snapshot.root,WindowOperation::Close);
        }
    }
    fn can_control(&self)->bool {
        self.session.as_ref().is_some_and(|session| {
            let info=session.info();
            session.mode==crate::screen::core::wire_hello::SessionMode::Control
                && info.hello.as_ref().is_some_and(|h|h.authorization.control!=crate::share::access_grant::CapabilityDecision::Off)
                && info.blocked.is_none() && !info.control_denied && !info.is_ended()
        })
    }
    fn metadata(&mut self,ctx:&egui::Context) {
        let snapshot=self.snapshot();
        let root=snapshot.as_ref().and_then(|s|s.members.iter().find(|m|m.identity==s.root));
        let app=root.map(|m|m.app.as_str()).filter(|name|!name.is_empty()).unwrap_or("Fernfenster");
        let title=format!("{app} – {}",self.device);
        if title!=self.title {ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));self.title=title;}
        if let Some(session)=&self.session {
            let info=session.info();
            if let Some(icon)=info.family_icon.clone() {
                if self.icon.as_ref().is_none_or(|old|!Arc::ptr_eq(old,&icon)) {
                    ctx.send_viewport_cmd(ViewportCommand::Icon(Some(icon.clone())));self.icon=Some(icon);
                }
            } else if self.icon.take().is_some() {
                ctx.send_viewport_cmd(ViewportCommand::Icon(None));
            }
            if let Some(icon)=root.and_then(|m|m.icon.as_ref()) {
                let mut icons=info.icons.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !icons.contains(&info.icon_peer,icon) {
                    icons.requests.request(crate::screen::core::app_icon::IconRequest {
                        target:crate::screen::core::app_icon::IconTarget::App {app_id:icon.app_id.clone()},icon:icon.clone() });
                }
            }
        }
    }
    fn menu(&mut self,ctx:&egui::Context) {
        let was_open=self.menu;
        self.local_block=was_open;
        egui::Area::new(egui::Id::new("family-menu-button")).anchor(egui::Align2::LEFT_TOP,[8.0,8.0])
            .order(egui::Order::Foreground).show(ctx,|ui| {
                let response=ui.button("Menü").on_hover_text("Fenstermenü · Strg+Alt+M");
                self.local_block|=response.hovered()||response.contains_pointer();
                if response.clicked(){self.menu=!self.menu;}
            });
        if was_open!=self.menu {if let Some(session)=&self.session {
            self.widget.release_input(session);super::family_viewer_session::disarm(session);
        }}
        if !self.menu{return;}
        let mut open=self.menu;
        egui::Window::new("Fenstermenü · Strg+Alt+M").id(egui::Id::new("family-menu"))
            .open(&mut open).collapsible(false).resizable(false).show(ctx,|ui| {
                ui.label(&self.title);
                if let Some(session)=&self.session {
                    let info=session.info();
                    ui.label(format!("Verbindung: {} · {:?}",info.path.as_deref().unwrap_or("wird hergestellt"),info.state));
                    if let Some(family)=&info.family {if let Some(notice)=&family.notice {ui.label(notice);}}
                    if let Some(reason)=info.reason.as_ref().or(info.notice.as_ref()) {ui.label(reason);}
                }
                let allowed=self.control&&self.can_control();
                if ui.add_enabled(self.target.control,egui::Checkbox::new(&mut self.control,"Steuern"))
                    .on_hover_text("Steuern benötigt eine mit Steuerrecht geöffnete Sitzung.").changed() {
                    if let Some(session)=&self.session {
                        self.widget.release_input(session);super::family_viewer_session::disarm(session);
                        session.send(ViewerFrame::ControlRequest(ControlRequest {action:if self.control {ControlAction::Take}else{ControlAction::Release}}));
                    }
                }
                if !self.target.control {ui.label("Zum Steuern in der Fensterliste „Steuern“ wählen und neu öffnen.");}
                if ui.add_enabled(allowed,egui::Button::new("Fensterfokus bestätigen")).clicked() {
                    if let (Some(session),Some(snapshot))=(&self.session,self.snapshot()) {
                        let identity=snapshot.focused_target().map(|t|t.identity).unwrap_or(snapshot.root);
                        super::family_viewer_session::operate(session,identity,WindowOperation::Focus);
                    }
                }
                ui.checkbox(&mut self.remote_resize,"Lokale Größe an Programm senden");
                let close=allowed&&self.snapshot().is_some_and(|s|s.members.iter().any(|m|m.identity==s.root&&m.operations.close));
                if ui.add_enabled(close,egui::Button::new("Programm schließen")).clicked(){self.close_program();}
                if !close {ui.label("Reguläres Schließen ist für das Hauptfenster aktuell nicht freigegeben.");}
                if ui.button("Fensterzustand aktualisieren").clicked(){if let Some(session)=&self.session{session.send(ViewerFrame::FamilySnapshot);}}
                ui.checkbox(&mut self.diagnostics,"Diagnose anzeigen");
                ui.separator();
                if ui.button("Verbindung trennen").clicked(){self.stop(ctx);}
            });
        self.menu=open;
    }
    fn resize(&mut self,display:FrameSize) {
        if self.last_display.is_none(){self.last_display=Some(display);self.resize_sent=Some(display);return;}
        if self.last_display!=Some(display) {self.last_display=Some(display);self.resize_at=Some(Instant::now());return;}
        if !self.remote_resize||!self.control||!self.can_control()
            ||self.resize_at.is_none_or(|at|at.elapsed()<Duration::from_millis(400))||self.resize_sent==Some(display){return;}
        let Some(snapshot)=self.snapshot()else{return;};
        let Some(root)=snapshot.members.iter().find(|m|m.identity==snapshot.root&&m.operations.resize)else{return;};
        let visible:Vec<_>=snapshot.members.iter().filter(|m|m.visible&&!m.minimized).collect();
        let width=visible.iter().map(|m|i64::from(m.geometry.x)+i64::from(m.geometry.width)).max().unwrap_or(0)
            -visible.iter().map(|m|i64::from(m.geometry.x)).min().unwrap_or(0);
        let height=visible.iter().map(|m|i64::from(m.geometry.y)+i64::from(m.geometry.height)).max().unwrap_or(0)
            -visible.iter().map(|m|i64::from(m.geometry.y)).min().unwrap_or(0);
        let wanted=FrameSize::new((i64::from(display.width)-(width-i64::from(root.geometry.width))).max(1) as u32,
            (i64::from(display.height)-(height-i64::from(root.geometry.height))).max(1) as u32);
        if let Some(session)=&self.session {super::family_viewer_session::operate(session,root.identity.clone(),
            WindowOperation::Resize {width:wanted.width,height:wanted.height});}
        self.resize_sent=Some(display);
    }
}
impl eframe::App for FamilyViewerApp {
    fn update(&mut self,ctx:&egui::Context,frame:&mut eframe::Frame) {
        let visibility=super::screen_visibility_os::sample(frame,ctx);
        if visibility.exit_chord&&self.fullscreen {
            self.fullscreen=false;ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
            ctx.input_mut(|i|i.events.clear());
        }
        ctx.data_mut(|d|d.insert_temp(egui::Id::new("screen_window_visibility"),visibility));
        if crate::app::update_stop_ui::close_requested(ctx){self.stop(ctx);return;}
        self.open(ctx);self.metadata(ctx);
        if ctx.input(|i|i.viewport().close_requested())&&!self.disconnect {
            if self.control&&self.can_control() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);self.close_program();self.menu=true;
            } else {self.stop(ctx);return;}
        }
        let chord=egui::KeyboardShortcut::new(egui::Modifiers::CTRL|egui::Modifiers::ALT,egui::Key::M);
        if ctx.input_mut(|i|i.consume_shortcut(&chord)){
            self.menu=!self.menu;
            if let Some(session)=&self.session {self.widget.release_input(session);super::family_viewer_session::disarm(session);}
        }
        self.menu(ctx);
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx,|ui| {
            if let Some(error)=&self.error {ui.label(error);return;}
            if self.session.is_none(){
                let devices=self.devices.lock();ui.spinner();
                ui.label(devices.error.as_deref().unwrap_or("Gespeicherte Geräteidentität wird geladen …"));return;
            }
            if let Some(session)=&self.session {
                let info=session.info().clone();
                let status=info.end.as_ref().map(|end|end.text.as_str()).or(info.reason.as_deref()).unwrap_or("Fensterverbindung wird hergestellt …");
                let lines=self.diagnostics.then(||super::screen_stats_overlay::stats_lines(&info,0));
                let params=ViewParams {fit:self.prefs.fit,control:self.control&&!self.menu&&!self.diagnostics&&!self.local_block&&self.can_control(),
                    dimmed:info.state!=SessionState::Streaming,status,stats:lines.as_deref()};
                let result=self.widget.show(ui,session,&params);
                if result.escape&&!self.control&&self.fullscreen {
                    self.fullscreen=false;ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
                for local in result.locals {match local {
                    super::screen_input_keys::LocalKey::Stats=>self.diagnostics=!self.diagnostics,
                    super::screen_input_keys::LocalKey::Fullscreen=>{
                        self.fullscreen=!self.fullscreen;ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
                    }
                    super::screen_input_keys::LocalKey::ExitFullscreen=>{
                        self.fullscreen=false;ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                    }
                    super::screen_input_keys::LocalKey::ToggleControl=>{self.menu=true;}
                }}
                self.resize(result.display);
            }
        });
        if self.diagnostics {if let Some(snapshot)=self.snapshot(){egui::Window::new("Fensterdiagnose").open(&mut self.diagnostics).show(ctx,|ui| {
            ui.label("Diagnose schließen, um das Programm zu steuern.");
            ui.label(format!("Revision {} · Layout {} · {} Fenster",snapshot.revision,snapshot.layout_revision,snapshot.members.len()));
            for member in &snapshot.members {ui.label(format!("{:?}: {} · {}×{} · Fokus {}",member.kind,member.title,
                member.geometry.width,member.geometry.height,member.focused));}
            for limitation in &snapshot.limitations {ui.label(limitation);}
        });}}
        if Instant::now()<self.initial_until {egui::Area::new(egui::Id::new("family-hint")).anchor(egui::Align2::CENTER_BOTTOM,[0.0,-12.0])
            .show(ctx,|ui|{ui.label("Fenstermenü: ⋮ oben links · Strg+Alt+M");});}
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
