//! `se screen windows DEVICE [--json]` (SCR-M4): the windows of another
//! device that can be shown as a source (`window:<id>`). The device's user
//! may be asked first („Fragen“); the command then waits without a time
//! limit and says so on stderr. JSON is the contract of the strand plan:
//! `{"windows":[{"id","title","app","pid","monitor","minimized","focused"}],
//! "apps":[{"app_id","name","pid"}],"list_source":"enum_windows|…",
//! "limits":[…]}`. Text lines are tab-separated: `list_source`, then `window`
//! (`id`, monitor, `minimized`, `focused`, app, title), `app` (id, name,
//! pid) and `limit` rows, `-` where a value is missing.
use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::Value;

use crate::screen::core::window_list::WindowList;

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct WindowsArgs {
    #[arg(
        help = "Device: share:// endpoint, name, id or fingerprint prefix",
        add = ArgValueCandidates::new(crate::cli::completions_screen::device_candidates)
    )]
    device: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// The JSON of a window list: the list as the wire carries it.
pub(crate) fn windows_value(list: &WindowList) -> Value {
    serde_json::to_value(list).unwrap_or_else(|_| serde_json::json!({}))
}

/// A text without tabs and line breaks, which separate the fields.
fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

fn dash(text: Option<&str>) -> String {
    text.filter(|text| !text.is_empty())
        .map_or_else(|| "-".to_string(), clean)
}

fn flag(set: bool, name: &str) -> String {
    String::from(if set { name } else { "-" })
}

/// The key of the list source (`enum_windows`, `ewmh`, …).
fn source_key(list: &WindowList) -> String {
    match serde_json::to_value(list.list_source) {
        Ok(Value::String(key)) => key,
        _ => "unknown".into(),
    }
}

pub(super) fn windows_text(list: &WindowList) -> String {
    let mut rows = vec![format!("list_source\t{}", source_key(list))];
    for window in &list.windows {
        rows.push(
            [
                "window".to_string(),
                clean(&window.id),
                dash(window.monitor.as_deref()),
                flag(window.minimized, "minimized"),
                flag(window.focused, "focused"),
                dash(Some(window.app.as_str())),
                clean(&window.title),
            ]
            .join("\t"),
        );
    }
    for app in &list.apps {
        let pid = app.pid.map(|pid| pid.to_string());
        rows.push(
            [
                "app".to_string(),
                clean(&app.app_id),
                clean(&app.name),
                dash(pid.as_deref()),
            ]
            .join("\t"),
        );
    }
    rows.extend(
        list.limits
            .iter()
            .map(|limit| format!("limit\t{}", clean(limit))),
    );
    let mut text = rows.join("\n");
    text.push('\n');
    text
}

pub(super) fn run(args: WindowsArgs) -> Result<i32, String> {
    let choice = super::select_device(&args.device)?;
    let list = crate::daemon::screen_windows(choice.target, super::pending_notice(choice.name))?;
    let value = windows_value(&list);
    super::output::print(args.json, &value, || windows_text(&list));
    Ok(0)
}
