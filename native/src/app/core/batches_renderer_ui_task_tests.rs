//! Controlled productive UI transitions; no GL/WGPU driver execution is claimed.
use super::*;
#[test]
fn batches_task_b6_renderer_cpu_handover_reuses_slot_and_rejects_retired_callback() {
    use super::super::super::screen_view_widget::ViewWidget;
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let texture = texture(&ctx, None);
    driver.upload();
    driver.complete_upload(texture.slot());
    let first = texture.slot().clone();
    let frame = texture.presented_frame().unwrap();
    let mut full = ViewWidget::new(false);
    full.adopt_texture(texture);
    let transfer = full.take_texture();
    assert!(Arc::ptr_eq(&first, transfer.slot()));
    assert!(!driver.draw(
        &first,
        frame,
        Arc::new(|| panic!("retired handover receipt"))
    ));
    let mut next = ViewWidget::new(false);
    next.adopt_texture(transfer);
    let returned = next.take_texture();
    assert!(Arc::ptr_eq(&first, returned.slot()));
    driver.owner.after_update(&ctx);
    driver.complete_retirement(&first);
    drop(returned);
    eprintln!("Kontrollierter Besitzer-/Callbackbeleg; realer Glow-FullOutput-/GL-Fence-Lauf ist hiervon nicht belegt.");
}

#[test]
fn batches_task_b6_renderer_full_widget_end_hidden_source_and_cpu_yuv_use_real_retirement_callers()
{
    use super::super::super::{
        screen_prefs::Fit,
        screen_session::{EndInfo, Picture, PictureData, ViewerSession},
        screen_view_widget::{ViewParams, ViewWidget},
        screen_visibility::WindowVisibility,
    };
    use crate::screen::core::{
        host_backend::{Chroma, YuvPicture},
        pixel_convert::YuvMatrix,
        session_state::SessionState,
    };
    for trigger in ["end", "hidden", "source", "cpu-yuv", "yuv-cpu"] {
        let ctx = egui::Context::default();
        let driver = ControlledDriver::install(&ctx);
        let shared = Shared::new(false, 0);
        shared.update(|info| info.state = SessionState::Streaming);
        let session = ViewerSession::fixture(shared.clone());
        let pool = Arc::new(MemoryPool::default());
        let memory = Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap());
        let mut widget = ViewWidget::new(true);
        let yuv = || {
            PictureData::Yuv(YuvPicture {
                y: vec![16; 4],
                y_stride: 2,
                uv: vec![128; 2],
                uv_stride: 2,
                chroma: Chroma::Nv12,
                matrix: YuvMatrix::Bt601,
            })
        };
        let show = |widget: &mut ViewWidget| {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(64.0, 64.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        widget.show(
                            ui,
                            &session,
                            &ViewParams {
                                fit: Fit::Actual,
                                control: false,
                                dimmed: false,
                                status: "fixture status",
                                stats: None,
                                hint: None,
                            },
                        );
                    });
                },
            );
            drop(output);
        };
        for id in 1..=2 {
            shared.put_picture(Picture {
                seq: id,
                presented: shared.begin_frame(1, id),
                size: FrameSize::new(2, 2),
                data: if trigger == "yuv-cpu" {
                    yuv()
                } else {
                    PictureData::Bgra(image())
                },
                memory: Some(memory.clone()),
            });
            show(&mut widget);
            driver.upload();
        }
        let slot = lock(&driver.owner.slots)
            .iter()
            .find(|slot| lock(&slot.state).resident.is_some())
            .unwrap()
            .clone();
        driver.complete_upload(&slot);
        let old = slot.frame().unwrap();
        match trigger {
            "end" => shared.end(EndInfo {
                text: "fixture end".into(),
                code: None,
            }),
            "hidden" => ctx.data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("screen_window_visibility"),
                    WindowVisibility {
                        minimized: true,
                        ..Default::default()
                    },
                )
            }),
            "source" => widget.source_changed(&session),
            _ => shared.put_picture(Picture {
                seq: 3,
                presented: shared.begin_frame(1, 3),
                size: FrameSize::new(2, 2),
                data: if trigger == "cpu-yuv" {
                    yuv()
                } else {
                    PictureData::Bgra(image())
                },
                memory: Some(memory.clone()),
            }),
        }
        show(&mut widget);
        driver.owner.after_update(&ctx);
        assert!(slot.frame().is_none(), "{trigger}");
        assert!(!driver.draw(&slot, old, Arc::new(|| panic!("stale widget receipt"))));
        assert!(lock(&slot.state).retiring.is_some(), "{trigger}");
        driver.complete_retirement(&slot);
        assert!(lock(&slot.state).retiring.is_none(), "{trigger}");
        drop(memory);
        widget.retire();
        drop(widget);
        driver.owner.after_update(&ctx);
        driver.owner.returned();
        assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
    }
}
#[test]
fn batches_task_b6_renderer_grid_offline_end_retry_and_drop_use_real_retirement_callers() {
    use super::super::super::{
        screen_devices::{DeviceEntry, Devices, GridScope},
        screen_session::{EndInfo, ViewerSession},
        screen_tile_grid::{Probe, ScreenTileGrid, Tile},
        screen_viewer_fake as fake,
    };
    use crate::share::{access_grant::CapabilityDecision, ShareStatus};
    for trigger in ["offline", "end", "retry", "drop"] {
        let ctx = egui::Context::default();
        let driver = ControlledDriver::install(&ctx);
        let pool = Arc::new(MemoryPool::default());
        let memory = Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap());
        let t = texture(&ctx, Some(memory.clone()));
        drop(memory);
        driver.upload();
        driver.complete_upload(t.slot());
        let slot = t.slot().clone();
        let shared = Shared::new(false, 0);
        let mut tile = Tile::new(DeviceEntry {
            target: fake::target(),
            name: "fixture".into(),
            room: None,
            online: true,
        });
        tile.texture = t;
        tile.session = Some(ViewerSession::fixture(shared.clone()));
        tile.probe = Probe::Done(super::super::super::screen_session::ProbeInfo {
            supported: true,
            view: CapabilityDecision::Allow,
        });
        tile.picture = Some(FrameSize::new(2, 2));
        let grid = ScreenTileGrid::new(
            GridScope::AllDevices,
            fake::Fake::new(CapabilityDecision::Allow).backend(),
            Devices::fixed(fake::profiles(ShareStatus::Offline, Some("device-c1"))),
        );
        match trigger {
            "offline" => {
                tile.entry.online = false;
                grid.step(&mut tile, &ctx, 2);
            }
            "end" => {
                shared.end(EndInfo {
                    text: "fixture end".into(),
                    code: None,
                });
                grid.step(&mut tile, &ctx, 2);
            }
            "retry" => super::super::super::screen_tile_face::retry(&mut tile),
            _ => {}
        }
        drop(tile);
        driver.owner.after_update(&ctx);
        assert!(slot.frame().is_none(), "{trigger}");
        assert!(pool.reserve(None, 1024 * 1024).is_err(), "{trigger}");
        driver.complete_retirement(&slot);
        assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok(), "{trigger}");
    }
}
