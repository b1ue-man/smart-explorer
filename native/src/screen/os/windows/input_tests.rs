//! Input on Windows (SCR-M6): the `SendInput` records of a batch (every
//! platform's Windows build), text reaching a real edit window and the block
//! of an administrator window (stage `windows-screen`, ignored, one thread).
use super::*;
use crate::screen::core::input_model::InputEvent;

const DESK: Area = Area {
    left: -1920,
    top: 0,
    width: 3840,
    height: 1080,
};

#[test]
fn screen_task_windows_input_records_map_positions_keys_text() {
    let right = Area {
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
    };
    // The right monitor's centre lies at three quarters of the desktop.
    let (x, y) = absolute(right, DESK, 32_768, 32_768);
    assert!((49_140..=49_160).contains(&x), "{x}");
    assert!((32_700..=32_800).contains(&y), "{y}");
    assert_eq!(absolute(right, DESK, 65_535, 65_535).0, 65_535);
    let mut sas = SasGuard::new();
    let events = [
        InputEvent::Key {
            hid_usage: 0xE0,
            down: true,
        },
        InputEvent::Key {
            hid_usage: 0xE2,
            down: true,
        },
        InputEvent::Key {
            hid_usage: 0x4C,
            down: true,
        },
        InputEvent::Text { text: "é".into() },
        InputEvent::Wheel { dx: 0, dy: -120 },
    ];
    let inputs = records(&events, right, DESK, &mut sas);
    // Ctrl and Alt go out, the Delete of Ctrl+Alt+Delete never.
    assert_eq!(inputs.len(), 2 + 2 + 1);
    // SAFETY: the records were written as keyboard and mouse inputs above.
    unsafe {
        assert_eq!(inputs[0].Anonymous.ki.wScan, 0x1D);
        assert_eq!(inputs[2].Anonymous.ki.wScan, u16::from(b'\xE9'));
        assert_eq!(inputs[2].Anonymous.ki.dwFlags, KEYEVENTF_UNICODE);
        assert_eq!(inputs[4].Anonymous.mi.dwFlags, MOUSEEVENTF_WHEEL);
    }
}

/// An edit window of this process, in front, that takes the text.
#[test]
#[ignore = "needs an interactive desktop (stage windows-screen)"]
fn screen_task_windows_sendinput_text_reaches_window() {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowTextW, PeekMessageW,
        SetForegroundWindow, ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOW,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    let class: Vec<u16> = "EDIT\0".encode_utf16().collect();
    let title: Vec<u16> = "\0".encode_utf16().collect();
    // SAFETY: a plain top-level edit window of this thread, destroyed below.
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            400,
            120,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    assert!(!window.is_null(), "no edit window");
    let pump = || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            // SAFETY: the message structure is a local; messages of this
            // thread's windows are handled in place.
            unsafe {
                let mut message: MSG = std::mem::zeroed();
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    // SAFETY: plain calls on the window created above.
    unsafe {
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
    }
    pump();
    set_target(&SourceId::Window((window as usize).to_string()));
    let events = [
        InputEvent::PointerAbs {
            x: 32_768,
            y: 32_768,
        },
        InputEvent::Button {
            button: PointerButton::Left,
            down: true,
        },
        InputEvent::Button {
            button: PointerButton::Left,
            down: false,
        },
        InputEvent::Text {
            text: "Hallo Ä".into(),
        },
    ];
    assert_eq!(inject(&events), Ok(()));
    pump();
    let mut text = [0u16; 64];
    // SAFETY: the buffer holds 64 units and its length is passed.
    let length = unsafe { GetWindowTextW(window, text.as_mut_ptr(), 64) };
    // SAFETY: created above, destroyed once.
    unsafe { DestroyWindow(window) };
    assert_eq!(
        String::from_utf16_lossy(&text[..length as usize]),
        "Hallo Ä"
    );
}

/// The runner starts an elevated window and brings it to the front
/// (`SE_SCREEN_ELEVATED_TASK=1`); without it the test is not run (NR N7).
#[test]
#[ignore = "needs an elevated window beside medium integrity (stage windows-screen)"]
fn screen_task_windows_elevated_blocked_reported() {
    if std::env::var_os("SE_SCREEN_ELEVATED_TASK").is_none() {
        eprintln!("not run: the runner started no elevated window");
        return;
    }
    set_target(&SourceId::Monitor(monitors::infos()[0].id.clone()));
    let move_only = [InputEvent::PointerAbs { x: 100, y: 100 }];
    assert_eq!(inject(&move_only), Err(ControlBlock::ElevatedWindow));
}

#[test]
fn screen_metadata_runtime_task_window_input_area_is_the_visible_frame() {
    // DWMWA_EXTENDED_FRAME_BOUNDS of a window at (100, 50), 800 x 600
    // visible pixels: the pointer range covers exactly that frame.
    let area = window_area(100, 50, 900, 650).unwrap();
    assert_eq!(
        area,
        Area {
            left: 100,
            top: 50,
            width: 800,
            height: 600,
        }
    );
    let desk = Area {
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
    };
    // The source origin is the frame's first visible pixel, not the
    // invisible resize border GetWindowRect may add to its left.
    let (x, y) = absolute(area, desk, 0, 0);
    assert_eq!(x, (100 * 65_535 / 1919) as i32);
    assert_eq!(y, (50 * 65_535 / 1079) as i32);
    assert_eq!(window_area(10, 10, 10, 20), None);
    assert_eq!(window_area(10, 20, 30, 5), None);
    assert_eq!(window_area(i32::MIN, 0, i32::MAX, 1), None);
}
