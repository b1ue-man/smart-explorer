//! Acceptance of the window list rule (SCR-M4): which windows Alt+Tab
//! shows, from a table of window facts; and the names of programs.
use super::*;

fn facts() -> WindowFacts {
    WindowFacts {
        visible: true,
        has_title: true,
        ..WindowFacts::default()
    }
}

#[test]
fn screen_task_windows_window_filter_alt_tab_rule() {
    struct Row {
        what: &'static str,
        facts: WindowFacts,
        shown: bool,
    }
    let rows = [
        Row {
            what: "an ordinary top-level window",
            facts: facts(),
            shown: true,
        },
        Row {
            what: "invisible",
            facts: WindowFacts {
                visible: false,
                ..facts()
            },
            shown: false,
        },
        Row {
            what: "without a title",
            facts: WindowFacts {
                has_title: false,
                ..facts()
            },
            shown: false,
        },
        Row {
            what: "cloaked (another virtual desktop, a suspended app)",
            facts: WindowFacts {
                cloaked: true,
                ..facts()
            },
            shown: false,
        },
        Row {
            what: "the shell window",
            facts: WindowFacts {
                shell: true,
                ..facts()
            },
            shown: false,
        },
        Row {
            what: "an owned window (a dialog)",
            facts: WindowFacts {
                has_owner: true,
                ..facts()
            },
            shown: false,
        },
        Row {
            what: "an owned window marked as an app window",
            facts: WindowFacts {
                has_owner: true,
                app_window: true,
                ..facts()
            },
            shown: true,
        },
        Row {
            what: "a tool window",
            facts: WindowFacts {
                tool_window: true,
                ..facts()
            },
            shown: false,
        },
        Row {
            what: "a tool window marked as an app window",
            facts: WindowFacts {
                tool_window: true,
                app_window: true,
                ..facts()
            },
            shown: true,
        },
        Row {
            what: "an owned tool window",
            facts: WindowFacts {
                has_owner: true,
                tool_window: true,
                ..facts()
            },
            shown: false,
        },
    ];
    for row in rows {
        assert_eq!(is_alt_tab_window(&row.facts), row.shown, "{}", row.what);
    }
}

#[test]
fn windows_app_names_are_file_names_without_extension() {
    assert_eq!(app_name(r"C:\Windows\System32\notepad.exe"), "notepad");
    assert_eq!(app_name(r"C:\Program Files\App\my.tool.exe"), "my.tool");
    assert_eq!(app_name("explorer"), "explorer");
    assert_eq!(app_name(r"C:\x\.hidden"), ".hidden");
    assert_eq!(app_name(""), "");
}
