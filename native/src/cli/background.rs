//! `se background`: the background worker settings of the desktop settings
//! page "Integration → Hintergrund" (`app/core/settings_background.rs`) and of
//! the Android `bg.*` methods — background sync at login, check interval,
//! pause, automatic pause, status and worker log. Every value is the control
//! file the GUI and the worker read; nothing here has its own store.
use clap::{Args, Subcommand};
use serde_json::{json, Value};

/// The check interval range of the desktop settings (`set_cadence_secs`
/// clamps to the same range).
const MIN_CADENCE_SECS: u64 = 2;
const MAX_CADENCE_SECS: u64 = 3600;

const BACKGROUND_HELP: &str = "\
The background worker runs saved sync setups on their schedule while the app is
closed. These are the settings of the app's \"Settings → Integration →
Background\" section and of Android's background settings; they take effect
for the running worker without a restart.

Examples:
  se background status --json
  se background enable
  se background cadence 60
  se background pause 2h
  se background pause forever
  se background resume
  se background autopause --battery true --metered false
  se background log --lines 50";

#[derive(Args)]
#[command(long_about = BACKGROUND_HELP)]
pub(super) struct BackgroundArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Show login start, worker state, pause, check interval and automatic pause")]
    Status {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Start background sync at login and start the worker now")]
    Enable,
    #[command(about = "Stop starting background sync at login (a running Share worker stays)")]
    Disable,
    #[command(
        about = "Set how often the worker looks for due setups, changes and devices (2-3600 seconds)"
    )]
    Cadence {
        #[arg(
            value_parser = clap::value_parser!(u64).range(MIN_CADENCE_SECS..=MAX_CADENCE_SECS),
            help = "Seconds between checks; shorter reacts faster and uses more CPU"
        )]
        seconds: u64,
    },
    #[command(about = "Pause all background syncs for a duration or without end")]
    Pause {
        #[arg(
            value_name = "DURATION",
            help = "90m, 2h, 1d, plain seconds, or `forever`"
        )]
        duration: String,
    },
    #[command(about = "End a pause")]
    Resume,
    #[command(about = "Pause automatically in battery saver mode or on a metered network")]
    Autopause(AutopauseArgs),
    #[command(about = "Print the end of the worker log")]
    Log {
        #[arg(long, default_value_t = 200, help = "Number of lines from the end")]
        lines: usize,
    },
}

#[derive(Args)]
struct AutopauseArgs {
    #[arg(long, help = "Pause while battery saver mode is on (true/false)")]
    battery: Option<bool>,
    #[arg(long, help = "Pause on a metered network (true/false)")]
    metered: Option<bool>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// A parsed pause duration.
#[derive(Debug, PartialEq, Eq)]
enum PauseSpan {
    Seconds(i64),
    Forever,
}

pub(super) fn run(args: BackgroundArgs) -> Result<i32, String> {
    match args.command.unwrap_or(Command::Status { json: false }) {
        Command::Status { json } => print_status(json),
        Command::Enable => enable(),
        Command::Disable => {
            crate::autostart::disable()
                .map_err(|error| format!("disable background sync at login: {error}"))?;
            println!("background sync at login off; a running Share worker may keep running");
            Ok(0)
        }
        Command::Cadence { seconds } => {
            crate::daemon::set_cadence_secs(seconds)
                .map_err(|error| format!("save the check interval: {error}"))?;
            println!("check_interval\t{seconds} s");
            Ok(0)
        }
        Command::Pause { duration } => {
            let saved = match parse_pause(&duration)? {
                PauseSpan::Forever => crate::daemon::pause_indefinite(),
                PauseSpan::Seconds(seconds) => crate::daemon::pause_for_secs(seconds),
            };
            saved.map_err(|error| format!("save the pause: {error}"))?;
            println!("paused\t{}", pause_text(crate::daemon::pause_remaining()));
            Ok(0)
        }
        Command::Resume => {
            crate::daemon::resume().map_err(|error| format!("end the pause: {error}"))?;
            println!("paused\tno");
            Ok(0)
        }
        Command::Autopause(args) => autopause(args),
        Command::Log { lines } => {
            println!("{}", crate::daemon::read_log_tail(lines.max(1)));
            Ok(0)
        }
    }
}

/// The desktop flow: the login entry first, then the worker; when the worker
/// cannot start safely the login entry is taken back.
fn enable() -> Result<i32, String> {
    crate::autostart::enable()
        .map_err(|error| format!("enable background sync at login: {error}"))?;
    if let Err(error) = crate::daemon::request_daemon_replacement() {
        let rollback = crate::autostart::disable()
            .err()
            .map(|rollback| format!("; taking back the login start failed: {rollback}"))
            .unwrap_or_default();
        return Err(format!(
            "background sync stays off: the worker could not be started safely: {error}{rollback}"
        ));
    }
    println!("background sync at login on; worker started");
    Ok(0)
}

/// Changes only the named conditions; a condition the platform cannot
/// observe stays as saved, as the desktop settings show it disabled.
fn autopause(args: AutopauseArgs) -> Result<i32, String> {
    let (battery, metered) = crate::daemon::autopause_flags()
        .map_err(|error| format!("read the automatic pause: {error}"))?;
    let support = crate::daemon::autopause_support();
    if args.battery.is_some_and(|wanted| wanted != battery) && !support.battery_saver {
        return Err(
            "pausing in battery saver mode is not supported on this platform; the saved value stays"
                .into(),
        );
    }
    if args.metered.is_some_and(|wanted| wanted != metered) && !support.metered {
        return Err(
            "pausing on a metered network is not supported on this platform; the saved value stays"
                .into(),
        );
    }
    if args.battery.is_some() || args.metered.is_some() {
        crate::daemon::set_autopause_flags(
            args.battery.unwrap_or(battery),
            args.metered.unwrap_or(metered),
        )
        .map_err(|error| format!("save the automatic pause: {error}"))?;
    }
    let value = autopause_value();
    if args.json {
        println!("{}", pretty(&value)?);
    } else {
        for line in autopause_lines(&value) {
            println!("{line}");
        }
    }
    Ok(0)
}

fn print_status(json: bool) -> Result<i32, String> {
    let value = status_value();
    if json {
        println!("{}", pretty(&value)?);
        return Ok(0);
    }
    println!(
        "sync_at_login\t{}",
        on_off(value["sync_at_login"].as_bool().unwrap_or(false))
    );
    if value["login_start_disabled_by_system"] == true {
        println!("sync_at_login_note\tthe desktop session skips the login entry");
    }
    let worker = match (
        value["worker_running"].as_bool().unwrap_or(false),
        value["heartbeat_age_seconds"].as_i64(),
    ) {
        (true, Some(age)) => format!("running (last report {age} s ago)"),
        (true, None) => "running (time of the last report unknown)".to_string(),
        (false, Some(age)) => format!("no current report (last {age} s ago)"),
        (false, None) => "no report".to_string(),
    };
    println!("worker\t{worker}");
    println!("paused\t{}", pause_text(crate::daemon::pause_remaining()));
    match value["check_interval_seconds"].as_u64() {
        Some(seconds) => println!("check_interval\t{seconds} s"),
        None => println!(
            "check_interval\tunreadable: {}",
            value["check_interval_seconds"]
        ),
    }
    for line in autopause_lines(&value["autopause"]) {
        println!("{line}");
    }
    match value["next_scheduled_run"].as_i64() {
        Some(at) => println!("next_scheduled_run\t{at}"),
        None => println!("next_scheduled_run\tnone"),
    }
    Ok(0)
}

/// The document of `se background status --json` and of the MCP `status`
/// tool. A control file that cannot be read appears as `{"error": …}`, like
/// the "nicht lesbar" line of the desktop settings.
pub(in crate::cli) fn status_value() -> Value {
    let (paused, paused_forever, remaining) = match crate::daemon::pause_remaining() {
        Ok(None) => (json!(false), false, None),
        Ok(Some(i64::MAX)) => (json!(true), true, None),
        Ok(Some(seconds)) => (json!(true), false, Some(seconds)),
        Err(error) => (json!({ "error": error.to_string() }), false, None),
    };
    let check_interval = match crate::daemon::cadence_secs() {
        Ok(seconds) => json!(seconds),
        Err(error) => json!({ "error": error.to_string() }),
    };
    json!({
        "sync_at_login": crate::autostart::is_enabled(),
        "login_start_disabled_by_system": crate::autostart::disabled_by_system(),
        "worker_running": crate::daemon::is_running(),
        "heartbeat_age_seconds": crate::daemon::last_heartbeat_age(),
        "paused": paused,
        "paused_forever": paused_forever,
        "pause_remaining_seconds": remaining,
        "paused_until": remaining.map(|seconds| now_secs().saturating_add(seconds)),
        "check_interval_seconds": check_interval,
        "autopause": autopause_value(),
        "next_scheduled_run": crate::daemon::next_scheduled_run(now_secs()),
    })
}

fn autopause_value() -> Value {
    let support = crate::daemon::autopause_support();
    match crate::daemon::autopause_flags() {
        Ok((battery, metered)) => json!({
            "battery_saver": battery,
            "metered": metered,
            "battery_saver_supported": support.battery_saver,
            "metered_supported": support.metered,
        }),
        Err(error) => json!({
            "error": error.to_string(),
            "battery_saver_supported": support.battery_saver,
            "metered_supported": support.metered,
        }),
    }
}

fn autopause_lines(value: &Value) -> Vec<String> {
    if let Some(error) = value["error"].as_str() {
        return vec![format!(
            "autopause\tunreadable, background sync held: {error}"
        )];
    }
    [
        ("autopause_battery_saver", "battery_saver"),
        ("autopause_metered", "metered"),
    ]
    .iter()
    .map(|(label, key)| {
        let supported = value[format!("{key}_supported")].as_bool().unwrap_or(false);
        format!(
            "{label}\t{}{}",
            on_off(value[*key].as_bool().unwrap_or(false)),
            if supported {
                ""
            } else {
                " (not observable on this platform)"
            }
        )
    })
    .collect()
}

fn pause_text(remaining: std::io::Result<Option<i64>>) -> String {
    match remaining {
        Ok(None) => "no".to_string(),
        Ok(Some(i64::MAX)) => "forever".to_string(),
        Ok(Some(seconds)) => format!(
            "until {} ({} min left)",
            now_secs().saturating_add(seconds),
            (seconds / 60).max(1)
        ),
        Err(error) => format!("unreadable, background sync held: {error}"),
    }
}

/// `90m`, `2h`, `1d`, `30s`, plain seconds, or `forever`; zero and negative
/// durations are refused like the Android `bg.pause`.
fn parse_pause(text: &str) -> Result<PauseSpan, String> {
    let text = text.trim();
    if ["forever", "indefinite", "unlimited"]
        .iter()
        .any(|word| text.eq_ignore_ascii_case(word))
    {
        return Ok(PauseSpan::Forever);
    }
    let invalid =
        || format!("invalid pause duration {text:?}: use 90m, 2h, 1d, seconds or forever");
    let (number, unit) = match text.char_indices().last() {
        Some((index, unit)) if unit.is_ascii_alphabetic() => (&text[..index], unit),
        Some(_) => (text, 's'),
        None => return Err(invalid()),
    };
    let factor = match unit.to_ascii_lowercase() {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return Err(invalid()),
    };
    let value: i64 = number.trim().parse().map_err(|_| invalid())?;
    match value.checked_mul(factor) {
        Some(seconds) if seconds > 0 => Ok(PauseSpan::Seconds(seconds)),
        _ => Err(invalid()),
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn pretty(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::{Cli, Command as RootCommand};
    use super::{parse_pause, status_value, Command, PauseSpan};

    fn parse(arguments: &[&str]) -> Result<Command, clap::Error> {
        let mut all = vec!["se", "background"];
        all.extend_from_slice(arguments);
        Cli::try_parse_from(all).map(|cli| match cli.command {
            RootCommand::Background(args) => args.command.expect("subcommand"),
            _ => panic!("wrong command"),
        })
    }

    #[test]
    fn luecken_task_background_pause_durations_match_the_gui_choices() {
        // The desktop buttons (2 h, 8 h, 24 h, ∞) and the Android ones (1 h, -1).
        assert_eq!(parse_pause("2h"), Ok(PauseSpan::Seconds(7_200)));
        assert_eq!(parse_pause("8H"), Ok(PauseSpan::Seconds(28_800)));
        assert_eq!(parse_pause("1d"), Ok(PauseSpan::Seconds(86_400)));
        assert_eq!(parse_pause("90m"), Ok(PauseSpan::Seconds(5_400)));
        assert_eq!(parse_pause("3600"), Ok(PauseSpan::Seconds(3_600)));
        assert_eq!(parse_pause("45s"), Ok(PauseSpan::Seconds(45)));
        assert_eq!(parse_pause("forever"), Ok(PauseSpan::Forever));
        for bad in [
            "",
            "0",
            "-1",
            "0h",
            "5x",
            "h",
            "two hours",
            "9223372036854775807d",
        ] {
            assert!(parse_pause(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn luecken_task_background_cli_parses_every_setting() {
        assert!(matches!(
            parse(&["cadence", "60"]).unwrap(),
            Command::Cadence { seconds: 60 }
        ));
        for bad in ["1", "3601", "-5", "x"] {
            assert!(parse(&["cadence", bad]).is_err(), "{bad} must be refused");
        }
        match parse(&["autopause", "--battery", "true", "--metered", "false"]).unwrap() {
            Command::Autopause(args) => {
                assert_eq!(args.battery, Some(true));
                assert_eq!(args.metered, Some(false));
            }
            _ => panic!("wrong subcommand"),
        }
        assert!(matches!(
            parse(&["status", "--json"]).unwrap(),
            Command::Status { json: true }
        ));
        let commands: [&[&str]; 4] = [&["enable"], &["disable"], &["resume"], &["pause", "2h"]];
        for command in commands {
            assert!(parse(command).is_ok(), "{command:?}");
        }
        assert!(matches!(
            parse(&["log", "--lines", "20"]).unwrap(),
            Command::Log { lines: 20 }
        ));
        assert!(Cli::try_parse_from(["se", "background"]).is_ok());
    }

    #[test]
    fn luecken_task_background_status_has_every_gui_value() {
        let value = status_value();
        for key in [
            "sync_at_login",
            "login_start_disabled_by_system",
            "worker_running",
            "heartbeat_age_seconds",
            "paused",
            "paused_forever",
            "pause_remaining_seconds",
            "paused_until",
            "check_interval_seconds",
            "autopause",
            "next_scheduled_run",
        ] {
            assert!(value.get(key).is_some(), "missing {key} in {value}");
        }
        for key in ["battery_saver_supported", "metered_supported"] {
            assert!(value["autopause"].get(key).is_some(), "missing {key}");
        }
    }
}
