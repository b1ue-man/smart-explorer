//! DSK-M6 checks of the X11 surface. The first (`desktop_task_linux_work_area_intersection`)
//! is pure; the others need Xvfb and the window managers of the suite stage
//! `linux-desktop: x11` (`SE_DESKTOP_TASK_X11=1`; `openbox`; `xfwm4` and
//! `xfdesktop` under `dbus-run-session`; `xdotool`; `xrandr`; the GUI program
//! with a graphics adapter, otherwise "nicht ausgeführt"):
//! `desktop_task_x11_window_type_desktop`, `desktop_task_x11_below_with_foreign_desktop`,
//! `desktop_task_x11_no_ewmh_unavailable`, `desktop_task_x11_monitors_two_screens` and
//! `desktop_task_cross_monitor_drag` (`desktop_task_x11_root_pixmap_background`:
//! `x11_pixmap_tests.rs`).
use std::time::Duration;

use eframe::egui::{Pos2, Rect};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::Window;

use super::super::x11_facts;
use super::super::x11_monitors::monitors;
use super::super::x11_task_env::{
    self as x11_task_env, client_windows, require_task, root_position, surface_windows, wait_for,
    window_info, WindowInfo, XServer,
};
use super::super::x11_task_surface::SurfaceProc;
use super::*;
use crate::app::desktop_surface_grid_ui::cell_rect;
use crate::desktop::core::layout::{Cell, ItemId, LayoutOp};
use crate::desktop::core::mechanism::{self, Mechanism, Reason};
use crate::desktop::core::protocol::SurfaceMessage;
use crate::desktop::os::Session;

#[test]
fn desktop_task_linux_work_area_intersection() {
    // Two workspaces with `x y width height` each; the current one counts.
    let values = [0, 0, 1920, 1040, 0, 30, 1920, 1010];
    assert_eq!(
        current_work_area(&values, Some(0)),
        Some([0, 0, 1920, 1040])
    );
    assert_eq!(
        current_work_area(&values, Some(1)),
        Some([0, 30, 1920, 1010])
    );
    assert_eq!(
        current_work_area(&values, Some(9)),
        Some([0, 0, 1920, 1040])
    );
    assert_eq!(current_work_area(&values, None), Some([0, 0, 1920, 1040]));
    assert_eq!(current_work_area(&[], Some(0)), None);
    assert_eq!(current_work_area(&[1, 2, 3], Some(0)), None);
    // A coordinate below zero arrives as a CARDINAL.
    let negative = [0xFFFF_FF38u32, 0, 100, 100];
    assert_eq!(
        current_work_area(&negative, None),
        Some([-200, 0, 100, 100])
    );

    // The work area of a panel at the bottom, cut with the monitor.
    assert_eq!(
        intersect_work_area([0, 0, 1920, 1040], [0, 0, 1920, 1080]),
        Some([0, 0, 1920, 1040])
    );
    // One work area over two monitors, a panel on top: cut with the second.
    let both = [0, 28, 3840, 1052];
    assert_eq!(
        intersect_work_area(both, [1920, 0, 3840, 1080]),
        Some([1920, 28, 3840, 1080])
    );
    assert_eq!(
        intersect_work_area(both, [0, 0, 1920, 1080]),
        Some([0, 28, 1920, 1080])
    );
    // A panel at the left edge of the first monitor leaves the second alone.
    assert_eq!(
        intersect_work_area([48, 0, 1872, 1080], [1920, 0, 3840, 1080]),
        None
    );
    // Monitors with a negative position.
    assert_eq!(
        intersect_work_area([-1920, 0, 3840, 1080], [-1920, 0, 0, 1080]),
        Some([-1920, 0, 0, 1080])
    );
    // The states per mechanism.
    assert_eq!(
        state_names(&Mechanism::X11Below),
        [
            "_NET_WM_STATE_BELOW",
            "_NET_WM_STATE_SKIP_TASKBAR",
            "_NET_WM_STATE_SKIP_PAGER",
            "_NET_WM_STATE_STICKY"
        ]
    );
    assert_eq!(state_names(&Mechanism::X11DesktopType).len(), 3);
    assert!(!state_names(&Mechanism::X11DesktopType).contains(&"_NET_WM_STATE_BELOW"));
    assert!(state_names(&Mechanism::Unavailable(Reason::NoEwmhDesktop)).is_empty());
}

const DESKTOP: &str = "_NET_WM_WINDOW_TYPE_DESKTOP";
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

/// Waits for `count` surface windows; the surface that ends on its own
/// (no graphics adapter) is "not run".
fn surface_windows_up(
    x: &XServer,
    processes: &mut [&mut SurfaceProc],
    count: usize,
) -> Vec<(Window, String)> {
    let (conn, root) = x.connect();
    let mut found = Vec::new();
    let seen = wait_for(Duration::from_secs(45), || {
        for process in processes.iter_mut() {
            process.not_run_if_ended();
        }
        found = surface_windows(&conn, root);
        found.len() >= count
    });
    if !seen {
        let processes: Vec<String> = processes.iter_mut().map(|p| p.describe()).collect();
        panic!(
            "{count} Fensterklasse(n) der Fläche fehlen: {found:?}; Fenster auf dem Server: {}; {}",
            x11_task_env::describe_windows(&conn, root),
            processes.join(" / ")
        );
    }
    found
}

/// The window's properties once `ready` accepts them.
fn settled(x: &XServer, window: Window, ready: impl Fn(&WindowInfo) -> bool) -> WindowInfo {
    let (conn, _) = x.connect();
    let mut last = window_info(&conn, window);
    wait_for(Duration::from_secs(10), || {
        last = window_info(&conn, window);
        ready(&last)
    });
    last
}

fn start_surface(x: &XServer, key: &str, extra: &[(&str, &str)]) -> SurfaceProc {
    let mut surface = SurfaceProc::start(x, &["--desktop-surface", "--monitor", key], extra);
    surface.greet(key, &[]);
    surface
}

#[test]
#[ignore = "braucht Xvfb und openbox (Stufe linux-desktop: x11)"]
fn desktop_task_x11_window_type_desktop() {
    require_task();
    let mut x = XServer::start("1280x800x24");
    x.start_window_manager("openbox", &["--sm-disable"]);
    let facts = session_facts::probe(&session_facts::SystemProbe);
    assert_eq!(
        mechanism::decide(
            &facts,
            crate::desktop::core::config::ForeignDesktopIcons::Keep
        ),
        Mechanism::X11DesktopType,
        "Openbox announces the window type and no foreign desktop window exists"
    );
    let key = monitors().first().expect("a monitor").key.clone();
    let mut surface = start_surface(&x, &key, &[]);
    let windows = surface_windows_up(&x, &mut [&mut surface], 1);
    let info = settled(&x, windows[0].0, |info| {
        info.types.iter().any(|name| name == DESKTOP)
            && info.state.iter().any(|name| name == "_NET_WM_STATE_STICKY")
    });
    assert!(info.types.iter().any(|name| name == DESKTOP), "{info:?}");
    for state in [
        "_NET_WM_STATE_SKIP_TASKBAR",
        "_NET_WM_STATE_SKIP_PAGER",
        "_NET_WM_STATE_STICKY",
    ] {
        assert!(
            info.state.iter().any(|name| name == state),
            "{state}: {info:?}"
        );
    }
    assert_eq!(info.desktop, Some(ALL_DESKTOPS), "all workspaces: {info:?}");
}

/// xfwm4 and xfdesktop under a session bus of their own.
fn start_xfce(x: &mut XServer) {
    let script = "xfwm4 --compositor=off & xfdesktop & wait";
    x.spawn("dbus-run-session", &["--", "sh", "-c", script]);
    x11_facts_ready(x, true);
}

/// Waits for a window manager and, with `foreign`, a desktop window that is
/// not ours.
fn x11_facts_ready(x: &XServer, foreign: bool) {
    let ready = wait_for(Duration::from_secs(45), || {
        x11_facts::read().is_some_and(|facts| {
            !facts.supported.is_empty() && facts.stacking_has_foreign_desktop == foreign
        })
    });
    if !ready {
        super::super::x11_task_env::not_run(&format!(
            "die Umgebung auf {} meldet kein Desktopfenster ({foreign})",
            x.display
        ));
    }
}

#[test]
#[ignore = "braucht Xvfb, xfwm4 und xfdesktop (Stufe linux-desktop: x11)"]
fn desktop_task_x11_below_with_foreign_desktop() {
    require_task();
    let mut x = XServer::start("1280x800x24");
    start_xfce(&mut x);
    let facts = session_facts::probe(&session_facts::SystemProbe);
    assert!(
        facts.foreign_desktop_window,
        "xfdesktop is a foreign desktop window"
    );
    assert_eq!(
        mechanism::decide(
            &facts,
            crate::desktop::core::config::ForeignDesktopIcons::Keep
        ),
        Mechanism::X11Below
    );
    let key = monitors().first().expect("a monitor").key.clone();
    let mut surface = start_surface(&x, &key, &[]);
    let windows = surface_windows_up(&x, &mut [&mut surface], 1);
    let desktop_window = windows[0].0;
    let info = settled(&x, desktop_window, |info| {
        info.state.iter().any(|name| name == "_NET_WM_STATE_BELOW")
            && info.desktop == Some(ALL_DESKTOPS)
    });
    for state in [
        "_NET_WM_STATE_BELOW",
        "_NET_WM_STATE_SKIP_TASKBAR",
        "_NET_WM_STATE_SKIP_PAGER",
    ] {
        assert!(
            info.state.iter().any(|name| name == state),
            "{state}: {info:?}"
        );
    }
    assert_eq!(info.desktop, Some(ALL_DESKTOPS), "{info:?}");

    // An ordinary window lies above the surface.
    let mut test_window = start_surface(&x, "task-window", &[("SE_DESKTOP_TEST_WINDOWED", "1")]);
    let windows = surface_windows_up(&x, &mut [&mut surface, &mut test_window], 2);
    let ordinary = windows
        .iter()
        .find(|(_, title)| title.contains("task-window"))
        .map(|(window, _)| *window)
        .expect("the test window");
    let (conn, root) = x.connect();
    let above = wait_for(Duration::from_secs(10), || {
        let order = client_windows(&conn, root, true);
        let at = |window: Window| order.iter().position(|known| *known == window);
        matches!((at(desktop_window), at(ordinary)), (Some(low), Some(high)) if low < high)
    });
    assert!(
        above,
        "the test window lies above the surface in _NET_CLIENT_LIST_STACKING"
    );
    // xfdesktop keeps running: its window is still there.
    assert!(x11_facts::read().is_some_and(|facts| facts.stacking_has_foreign_desktop));
    assert!(x.helper_running(0), "xfdesktop and xfwm4 still run");
}

#[test]
#[ignore = "braucht Xvfb (Stufe linux-desktop: x11)"]
fn desktop_task_x11_no_ewmh_unavailable() {
    require_task();
    let _x = XServer::start("1024x768x24");
    // No window manager: no `_NET_SUPPORTED`.
    let facts = session_facts::probe(&session_facts::SystemProbe);
    assert_eq!(
        facts.display,
        crate::desktop::core::mechanism::DisplayServer::X11
    );
    assert_eq!(facts.net_supported, None);
    assert!(!facts.foreign_desktop_window);
    let session = Session::probe(None);
    assert_eq!(
        session.mechanism,
        Mechanism::Unavailable(Reason::NoEwmhDesktop)
    );
    let reason = session
        .unavailable
        .iter()
        .find(|(part, _)| *part == crate::desktop::core::state::Part::Surface)
        .map(|(_, text)| text.clone())
        .expect("the surface part is unavailable");
    assert!(
        reason.contains("_NET_WM_WINDOW_TYPE_DESKTOP") && reason.contains("_NET_WM_STATE_BELOW"),
        "{reason}"
    );
    assert!(session.unavailable_codes.contains(&(
        crate::desktop::core::state::Part::Surface,
        "no_ewmh_desktop"
    )));
    // The shell starts no surface then.
    let wanted = crate::desktop::os::supervisor_surface::wanted_surfaces(
        &session.mechanism,
        true,
        &monitors(),
        None,
    );
    assert!(wanted.is_empty());
}

/// Two RandR monitors side by side: the left one takes the output `screen`
/// (so the server drops its automatic monitor), the right one has none.
fn two_monitors(x: &XServer) -> String {
    let output = monitors().first().expect("a monitor").key.clone();
    assert!(
        x.run(
            "xrandr",
            &[
                "--setmonitor",
                "left",
                "800/211x600/158+0+0",
                output.as_str()
            ]
        ),
        "xrandr --setmonitor left"
    );
    assert!(
        x.run(
            "xrandr",
            &["--setmonitor", "right", "800/211x600/158+800+0", "none"]
        ),
        "xrandr --setmonitor right"
    );
    output
}

/// The server's own list of monitors, for the message of a failed check.
fn server_monitors(x: &XServer) -> String {
    format!(
        "; xrandr --listmonitors:\n{}",
        x.output("xrandr", &["--listmonitors"])
    )
}

#[test]
#[ignore = "braucht Xvfb, openbox und xrandr (Stufe linux-desktop: x11)"]
fn desktop_task_x11_monitors_two_screens() {
    require_task();
    let mut x = XServer::start("1600x600x24");
    let output = two_monitors(&x);
    let listed = monitors();
    assert_eq!(listed.len(), 2, "{listed:?}{}", server_monitors(&x));
    assert_eq!(listed[0].rect, Some([0, 0, 800, 600]));
    assert_eq!(listed[1].rect, Some([800, 0, 1600, 600]));
    assert_eq!(listed[0].key, output, "the name of the first output");
    assert_eq!(
        listed[1].key, "right",
        "a monitor without output: its own name"
    );
    assert_eq!(listed.iter().filter(|monitor| monitor.primary).count(), 1);
    assert!(listed[0].primary, "no primary marked: the first one");
    let names = x11_monitors_names();
    assert_eq!(
        names[1],
        ("right".to_string(), "right (800×600)".to_string())
    );

    // One surface per monitor, each at its monitor's origin.
    x.start_window_manager("openbox", &["--sm-disable"]);
    let mut left = start_surface(&x, &listed[0].key, &[]);
    let mut right = start_surface(&x, &listed[1].key, &[]);
    let windows = surface_windows_up(&x, &mut [&mut left, &mut right], 2);
    let (conn, root) = x.connect();
    let mut lefts: Vec<i16> = windows
        .iter()
        .filter_map(|(window, _)| root_position(&conn, root, *window))
        .map(|(x, _)| x)
        .collect();
    lefts.sort_unstable();
    assert_eq!(lefts, [0, 800], "one surface at each monitor");
    // The work area of each monitor lies inside it.
    for monitor in &listed {
        let area = work_area(&monitor.key).or(monitor.rect).unwrap();
        let rect = monitor.rect.unwrap();
        assert!(
            area[0] >= rect[0] && area[2] <= rect[2],
            "{area:?} in {rect:?}"
        );
    }
}

fn x11_monitors_names() -> Vec<(String, String)> {
    super::super::x11_monitors::monitor_names()
}

#[test]
#[ignore = "braucht Xvfb, openbox, xrandr und xdotool (Stufe linux-desktop: x11)"]
fn desktop_task_cross_monitor_drag() {
    require_task();
    let mut x = XServer::start("1600x600x24");
    two_monitors(&x);
    let listed = monitors();
    assert_eq!(listed.len(), 2, "{listed:?}{}", server_monitors(&x));
    x.start_window_manager("openbox", &["--sm-disable"]);
    let mut left = SurfaceProc::start(&x, &["--desktop-surface", "--monitor", &listed[0].key], &[]);
    left.greet(&listed[0].key, &["a.txt"]);
    let mut right = start_surface(&x, &listed[1].key, &[]);
    surface_windows_up(&x, &mut [&mut left, &mut right], 2);

    // The tile in the first cell of the left monitor, dragged over the edge.
    let area = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
    let tile = cell_rect(area, Cell { col: 0, row: 0 }).center();
    let (tx, ty) = (tile.x as i32, tile.y as i32);
    let pointer = |args: &[&str]| assert!(x.run("xdotool", args), "xdotool {args:?}");
    let pause = || std::thread::sleep(Duration::from_millis(300));
    std::thread::sleep(Duration::from_secs(2));
    let row = ty.to_string();
    pointer(&["mousemove", tx.to_string().as_str(), row.as_str()]);
    pause();
    pointer(&["mousedown", "1"]);
    pause();
    for step in [20, 150, 400, 700, 900, 1100] {
        pointer(&["mousemove", (tx + step).to_string().as_str(), row.as_str()]);
        pause();
    }
    pointer(&["mouseup", "1"]);
    let moved = left.wait_message(Duration::from_secs(15), |message| {
        matches!(
            message,
            SurfaceMessage::Layout {
                op: LayoutOp::MoveToMonitor { .. },
                ..
            }
        )
    });
    match moved {
        Some(SurfaceMessage::Layout {
            op: LayoutOp::MoveToMonitor { item, monitor },
            ..
        }) => {
            assert_eq!(
                item,
                ItemId::File {
                    name: "a.txt".into()
                }
            );
            assert_eq!(monitor, listed[1].key, "the monitor under the pointer");
        }
        other => panic!(
            "kein Verschieben auf den zweiten Monitor (Annahme A31: Zeigerpositionen \
             außerhalb des Fensters während des Ziehens): {other:?}"
        ),
    }
}
