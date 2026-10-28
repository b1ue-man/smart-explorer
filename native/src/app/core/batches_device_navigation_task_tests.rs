//! Targeted G8/G9 product UI boundaries, without a daemon or real program launch.
use super::*;
use crate::app::desktop_surface_actions::{DesktopOp, SurfaceActions};
use crate::app::desktop_surface_grid_ui::{show_surface, SurfaceState};
use crate::app::desktop_surface_host::{NativeWindow, SurfaceHost};
use crate::app::desktop_surface_view_model::{Outbox, ViewModel};
use crate::desktop::core::config::{Background, BackgroundKind, DesktopConfig, SystemItems};
use crate::desktop::core::protocol::SurfaceMessage;
use crate::desktop::os::open_target::OpenRequest;
use crate::desktop::os::Monitor;
use std::path::PathBuf;

struct Sink;
impl Outbox for Sink {
    fn send(&self, _: SurfaceMessage) -> bool {
        true
    }
}

struct Host;
impl SurfaceHost for Host {
    fn monitor_key(&self) -> &str {
        "device-navigation"
    }
    fn monitors(&self) -> &[Monitor] {
        &[]
    }
    fn work_area(&self) -> Option<[i32; 4]> {
        None
    }
    fn native_window(&self) -> Option<NativeWindow> {
        None
    }
    fn request_repaint(&self) {}
    fn take_wallpaper_change(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct Actions {
    calls: usize,
}
impl SurfaceActions for Actions {
    fn open(&mut self, _: OpenRequest) {
        self.calls += 1;
    }
    fn run(&mut self, _: DesktopOp) {
        self.calls += 1;
    }
    fn set_clipboard(&mut self, _: &egui::Context, _: Vec<PathBuf>, _: bool) {
        self.calls += 1;
    }
    fn clipboard(&mut self, _: Option<&str>) -> Option<(Vec<PathBuf>, bool)> {
        self.calls += 1;
        None
    }
    fn take_notices(&mut self) -> Vec<String> {
        Vec::new()
    }
}

fn direct() -> PeerOpenTarget {
    PeerOpenTarget::Direct {
        contact_id: "saved-direct".into(),
    }
}

fn room() -> PeerOpenTarget {
    PeerOpenTarget::RoomDevice {
        room_id: "saved-room".into(),
        device_id: "remote-device".into(),
    }
}

fn request() -> LaunchRequest {
    LaunchRequest {
        request_id: "immutable-request".to_string().try_into().unwrap(),
        target_device_id: "remote-device".to_string().try_into().unwrap(),
        app_id: "app-id".to_string().try_into().unwrap(),
        user_id: "remote-user".to_string().try_into().unwrap(),
        session_id: "login-incarnation".to_string().try_into().unwrap(),
        catalog_revision: "catalog-revision".into(),
    }
}

fn state(target: &PeerOpenTarget) -> State {
    let request = request();
    let window = WindowBinding {
        app_id: request.app_id.clone(),
        user_id: request.user_id.clone(),
        session_id: request.session_id.clone(),
        window_id: "family-lease".into(),
        generation: 17,
    };
    State {
        restored: true,
        saved: Some(store::Saved {
            target: target.clone(),
            request: request.clone(),
            name: "Saved app".into(),
        }),
        receipt: Some(LaunchReceipt {
            request,
            launch_id: "launch-evidence".into(),
            dispatch_id: "dispatch-evidence".into(),
            state: LaunchState::Started,
            reason: "confirmed receipt".into(),
            pid: None,
            window: Some(window),
        }),
        node: Some("authenticated-node".into()),
        // Design 6 F6: a confirmed window opens by itself once; this one
        // was opened already, so drawing shows that instead of opening.
        opened: Some(self::request()),
        ..State::default()
    }
}

fn seed(ctx: &egui::Context, target: &PeerOpenTarget, state: State) {
    let id = egui::Id::new(("device-apps", target.endpoint_prefix(), None::<&str>));
    ctx.data_mut(|data| data.insert_temp(id, Arc::new(Mutex::new(state))));
}

fn text_shapes(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, result: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(text) => result.push(text.galley.text().to_string()),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, result)),
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in shapes {
        walk(&shape.shape, &mut result);
    }
    result
}

/// This is the same common product entrypoint used by eframe and OutputView::run_ui.
fn frame(
    ctx: &egui::Context,
    view: &mut ViewModel,
    ui: &mut SurfaceState,
    actions: &mut Actions,
) -> Vec<String> {
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1800.0, 1000.0),
            )),
            ..Default::default()
        },
        |ctx| show_surface(ctx, view, ui, &Host, actions),
    );
    text_shapes(&output.shapes)
}

fn view() -> ViewModel {
    let mut config = DesktopConfig::initial(SystemItems::initial(None, None));
    config.background = Background {
        kind: BackgroundKind::Color,
        value: Some("#102030".into()),
    };
    ViewModel::new(
        "device-navigation",
        Box::new(Sink),
        false,
        config,
        Vec::new(),
    )
}

#[test]
fn batches_task_device_navigation_surface_draws_picker_and_p4_errors() {
    install_window_opener();
    install_window_opener();
    assert!(OPENER.get().is_some());
    let ctx = egui::Context::default();
    let mut view = view();
    let mut ui = SurfaceState::new(false);
    let mut actions = Actions::default();
    seed(&ctx, &direct(), state(&direct()));
    open_surface(&ctx, direct(), "Saved direct".into());
    open_surface(&ctx, direct(), "Duplicate same identity".into());
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("family-open-error"),
            "Current generation refused".to_string(),
        )
    });
    // egui windows may spend the first frame measuring their content.
    frame(&ctx, &mut view, &mut ui, &mut actions);
    let text = frame(&ctx, &mut view, &mut ui, &mut actions);
    assert_eq!(
        text.iter()
            .filter(|text| text.as_str() == "Programme · Saved direct")
            .count(),
        1
    );
    assert!(text
        .iter()
        .any(|text| text == "Programmfenster wurde geöffnet."));
    assert!(text.iter().any(|text| text == "Current generation refused"));
    assert!(!text
        .iter()
        .any(|text| text.contains("Anschluss an die aktuelle Fernfensterauswahl")));
    assert!(!text
        .iter()
        .any(|text| text.contains("Duplicate same identity")));
    let saved = ctx
        .data(|data| {
            data.get_temp::<Shared>(egui::Id::new((
                "device-apps",
                direct().endpoint_prefix(),
                None::<&str>,
            )))
        })
        .unwrap();
    assert_eq!(
        saved.lock().unwrap().saved.as_ref().unwrap().request,
        request()
    );
    assert_eq!(
        actions.calls, 0,
        "Drawing must not invoke surface file actions"
    );
}

#[test]
fn batches_task_device_navigation_surface_keeps_direct_room_and_output_contexts_separate() {
    let first = egui::Context::default();
    let second = egui::Context::default();
    for target in [direct(), room()] {
        seed(&first, &target, state(&target));
        open_surface(&first, target.clone(), target.endpoint_prefix());
    }
    seed(&second, &direct(), state(&direct()));
    open_surface(&second, direct(), "Second output".into());
    let mut first_view = view();
    let mut first_ui = SurfaceState::new(false);
    let mut second_view = view();
    let mut second_ui = SurfaceState::new(false);
    let mut actions = Actions::default();
    frame(&first, &mut first_view, &mut first_ui, &mut actions);
    let first_text = frame(&first, &mut first_view, &mut first_ui, &mut actions);
    frame(&second, &mut second_view, &mut second_ui, &mut actions);
    let second_text = frame(&second, &mut second_view, &mut second_ui, &mut actions);
    for target in [direct(), room()] {
        assert!(first_text
            .iter()
            .any(|text| text == &format!("Programme · {}", target.endpoint_prefix())));
    }
    assert!(!first_text.iter().any(|text| text.contains("Second output")));
    assert!(second_text
        .iter()
        .any(|text| text == "Programme · Second output"));
    assert!(!second_text
        .iter()
        .any(|text| text.contains("share://room/")));
    assert_eq!(actions.calls, 0);
}

#[test]
fn batches_task_device_navigation_confirmed_receipt_keeps_identity_and_rejects_invalid_bindings() {
    for target in [direct(), room()] {
        let baseline = state(&target);
        let result = confirmed(&target, &baseline).unwrap();
        assert_eq!(result.target, target);
        assert_eq!(result.request, request());
        assert_eq!(result.node_id, "authenticated-node");
        assert_eq!(result.window.generation, 17);
        for denied in [
            LaunchState::OutcomeUnknown,
            LaunchState::ConsentRequired,
            LaunchState::Denied,
            LaunchState::Unsupported,
        ] {
            let mut invalid = state(&target);
            invalid.receipt.as_mut().unwrap().state = denied;
            assert!(confirmed(&target, &invalid).is_none());
        }
        let mut invalid = state(&target);
        invalid.node = None;
        assert!(confirmed(&target, &invalid).is_none());
        for field in 0..5 {
            let mut invalid = state(&target);
            let binding = invalid.receipt.as_mut().unwrap().window.as_mut().unwrap();
            match field {
                0 => binding.app_id = "different-app".to_string().try_into().unwrap(),
                1 => binding.user_id = "different-user".to_string().try_into().unwrap(),
                2 => binding.session_id = "different-login".to_string().try_into().unwrap(),
                3 => binding.window_id.clear(),
                _ => binding.generation = 0,
            }
            assert!(confirmed(&target, &invalid).is_none());
        }
    }
}
