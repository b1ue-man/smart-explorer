use super::super::screen_input_map::{Geometry, InputMapper};
use super::*;
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::pixel_convert::FrameSize;

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn batches_task_b6_exit_chord_stays_local_and_releases_host_modifiers() {
    let geometry = Geometry {
        rect: eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(100.0, 100.0),
        ),
        size: FrameSize::new(100, 100),
    };
    let mut mapper = InputMapper::new(InputCapabilities {
        keyboard: true,
        ..Default::default()
    });
    let mods = Modifiers {
        ctrl: true,
        alt: true,
        shift: true,
        ..Modifiers::NONE
    };
    mapper.process(&[], mods, &geometry);
    let pressed = mapper.take_batch().unwrap();
    assert_eq!(pressed.len(), 3);
    let chord = key(Key::F, true, mods);
    assert_eq!(local_keys(&[chord.clone()]), vec![LocalKey::ExitFullscreen]);
    mapper.process(&[chord], mods, &geometry);
    let released = mapper.take_batch().unwrap();
    for name in ["ctrl_left", "shift_left", "alt_left"] {
        let usage = key_usage(name).unwrap();
        assert!(pressed.iter().any(|event| matches!(event,
            InputEvent::Key { hid_usage, down: true } if *hid_usage == usage)));
        assert!(released.iter().any(|event| matches!(event,
            InputEvent::Key { hid_usage, down: false } if *hid_usage == usage)));
    }
    assert_eq!(released.len(), 3);
    mapper.process(&[key(Key::F, false, mods)], mods, &geometry);
    assert!(mapper.take_batch().is_none());
    mapper.process(&[], Modifiers::NONE, &geometry);
    mapper.process(
        &[
            key(Key::Enter, true, Modifiers::NONE),
            key(Key::Enter, false, Modifiers::NONE),
        ],
        Modifiers::NONE,
        &geometry,
    );
    let resumed = mapper.take_batch().unwrap();
    assert_eq!(resumed.len(), 2);
    assert!(resumed.iter().all(|event| matches!(event,
        InputEvent::Key { hid_usage, .. } if *hid_usage == key_usage("enter").unwrap())));
    mapper.release_all();
    assert!(mapper.take_batch().is_none());
}
