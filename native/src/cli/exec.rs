use clap::Args;
use clap_complete::ArgValueCandidates;
use std::collections::BTreeMap;
use std::io::{Read, Write};

const EXEC_HELP: &str = "\
Run a command as the Smart Explorer user's unrestricted shell authority on one
explicitly authorized Share peer. File access does not imply Exec access.

The target may be omitted when exactly one ready peer exists. Otherwise the
command prints every valid target and an immediately usable example.

Examples:
  se exec Laptop -- uname -a
  se exec share://direct/peer-id -- powershell -NoProfile -Command Get-Date
  se exec Laptop --shell 'printf \"%s\\n\" \"$HOME\"'
  se exec Laptop --cwd /srv --env MODE=production -- ./deploy --force

There is no default runtime or output limit. Use --timeout or --max-output only
when you explicitly want one. Ctrl+C cancels the contained remote process tree.";

#[derive(Args)]
#[command(long_about = EXEC_HELP)]
pub(super) struct ExecArgs {
    #[arg(
        help = "Optional peer selector or share:// endpoint; omit when exactly one peer is ready",
        add = ArgValueCandidates::new(crate::cli::completions::exec_target_candidates)
    )]
    pub(super) target: Option<String>,
    #[arg(
        long,
        help = "Remote working directory; defaults to the remote user's home"
    )]
    pub(super) cwd: Option<String>,
    #[arg(
        long,
        default_value_t = 0,
        help = "Remote timeout in seconds; 0 means unlimited"
    )]
    pub(super) timeout: u64,
    #[arg(
        long,
        default_value_t = 0,
        help = "Maximum combined stdout/stderr bytes; 0 means unlimited"
    )]
    pub(super) max_output: u64,
    #[arg(
        long = "env",
        value_name = "NAME=VALUE",
        help = "Set one remote environment value; repeatable"
    )]
    pub(super) env: Vec<String>,
    #[arg(
        long,
        conflicts_with = "argv",
        help = "Run the exact string through the remote user's shell"
    )]
    pub(super) shell: Option<String>,
    #[arg(
        last = true,
        num_args = 1..,
        allow_hyphen_values = true,
        help = "Program and literal arguments after --"
    )]
    pub(super) argv: Vec<String>,
}

pub(super) fn run(args: ExecArgs) -> Result<i32, String> {
    let target = select_target(args.target.as_deref())?;
    let command = match (args.shell, args.argv.split_first()) {
        (Some(command), None) if !command.trim().is_empty() => {
            crate::share::ExecCommand::Shell { command }
        }
        (None, Some((program, arguments))) => crate::share::ExecCommand::Argv {
            program: program.clone(),
            args: arguments.to_vec(),
        },
        (Some(_), Some(_)) => return Err("use either --shell or argv after --, not both".into()),
        _ => {
            return Err(
                "missing command; use `se exec TARGET -- PROGRAM ...` or `se exec TARGET --shell COMMAND`"
                    .into(),
            )
        }
    };
    let start = crate::share::ExecStart {
        exec_id: crate::share::ExecId::generate().map_err(|error| error.to_string())?,
        command,
        cwd: args.cwd,
        env: parse_environment(args.env)?,
        timeout_ms: (args.timeout > 0).then(|| args.timeout.saturating_mul(1_000)),
        max_output_bytes: (args.max_output > 0).then_some(args.max_output),
    };
    start.validate().map_err(|error| error.to_string())?;
    execute(target, start)
}

fn execute(
    target: crate::share::PeerOpenTarget,
    start: crate::share::ExecStart,
) -> Result<i32, String> {
    let mut session = crate::daemon::connect_exec(target, start)?;
    let stdin_input = session.take_input().map_err(|error| error.to_string())?;
    let cancel_input = stdin_input.clone();
    ctrlc::set_handler(move || {
        let _ = cancel_input.cancel();
    })
    .map_err(|error| format!("Ctrl+C handler: {error}"))?;
    std::thread::Builder::new()
        .name("se-exec-stdin".into())
        .spawn(move || stream_stdin(stdin_input))
        .map_err(|error| format!("stdin worker: {error}"))?;

    loop {
        let event = match session.next_event() {
            Ok(event) => event,
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "se: remote execution transport: {error}");
                return Ok(125);
            }
        };
        match event {
            crate::daemon::ExecIpcEvent::Authorized(_) | crate::daemon::ExecIpcEvent::Started => {}
            crate::daemon::ExecIpcEvent::Stdout(bytes) => write_all(std::io::stdout(), &bytes)?,
            crate::daemon::ExecIpcEvent::Stderr(bytes) => write_all(std::io::stderr(), &bytes)?,
            crate::daemon::ExecIpcEvent::Failed(error) => {
                let _ = writeln!(std::io::stderr(), "se: {}: {}", error.code, error.message);
                return Ok(125);
            }
            crate::daemon::ExecIpcEvent::Terminal(terminal) => {
                if terminal.output_truncated {
                    let _ = writeln!(
                        std::io::stderr(),
                        "se: remote output was truncated by the explicit --max-output limit"
                    );
                }
                return Ok(exit_code(&terminal));
            }
        }
    }
}

fn stream_stdin(input: crate::daemon::ExecIpcInput) {
    let mut stdin = std::io::stdin();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        match stdin.read(&mut bytes) {
            Ok(0) => {
                let _ = input.eof();
                return;
            }
            Ok(read) => {
                if input.stdin(&bytes[..read]).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                let _ = input.cancel();
                return;
            }
        }
    }
}

fn write_all(mut writer: impl Write, bytes: &[u8]) -> Result<(), String> {
    writer
        .write_all(bytes)
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

fn exit_code(terminal: &crate::share::ExecTerminal) -> i32 {
    use crate::share::ExecTerminalKind;
    match terminal.kind {
        ExecTerminalKind::Exited => terminal
            .exit_code
            .or_else(|| terminal.signal.map(|signal| 128 + signal))
            .unwrap_or(1),
        ExecTerminalKind::TimedOut => 124,
        ExecTerminalKind::Cancelled => 130,
        ExecTerminalKind::Failed | ExecTerminalKind::Revoked | ExecTerminalKind::Disconnected => {
            125
        }
    }
}

fn parse_environment(values: Vec<String>) -> Result<BTreeMap<String, String>, String> {
    let mut environment = BTreeMap::new();
    for value in values {
        let (name, value) = value
            .split_once('=')
            .ok_or_else(|| format!("invalid --env {value:?}; expected NAME=VALUE"))?;
        if name.is_empty() || name.contains('\0') || name.contains('=') || value.contains('\0') {
            return Err(format!("invalid --env name/value: {name:?}"));
        }
        environment.insert(name.to_string(), value.to_string());
    }
    Ok(environment)
}

#[derive(Clone)]
struct TargetChoice {
    target: crate::share::PeerOpenTarget,
    endpoint: String,
    label: String,
    selectors: Vec<String>,
    ready: bool,
}

fn select_target(selector: Option<&str>) -> Result<crate::share::PeerOpenTarget, String> {
    let snapshot = crate::daemon::drain_share_worker_events()?;
    let choices = target_choices(&snapshot.profiles);
    if let Some(selector) = selector.map(str::trim).filter(|value| !value.is_empty()) {
        if let Some((target, path)) = crate::share::PeerOpenTarget::from_endpoint(selector) {
            if path != "/" {
                return Err("Exec endpoints cannot contain a path; use --cwd instead".into());
            }
            return Ok(target);
        }
        let matches: Vec<_> = choices
            .iter()
            .filter(|choice| {
                choice
                    .selectors
                    .iter()
                    .any(|candidate| selector_matches(selector, candidate))
            })
            .collect();
        return match matches.as_slice() {
            [choice] => Ok(choice.target.clone()),
            [] => Err(format_choices(
                &choices,
                &format!("peer not found: {selector}"),
            )),
            _ => Err(format_choices(&matches, "peer selector is ambiguous")),
        };
    }
    let ready: Vec<_> = choices.iter().filter(|choice| choice.ready).collect();
    match ready.as_slice() {
        [choice] => Ok(choice.target.clone()),
        [] => Err(format_choices(&choices, "no ready Exec peer was found")),
        _ => Err(format_choices(
            &ready,
            "multiple Exec peers are ready; choose one",
        )),
    }
}

fn target_choices(profiles: &crate::share::ShareProfiles) -> Vec<TargetChoice> {
    let mut choices = Vec::new();
    let now = crate::share::core_now_secs();
    for contact in &profiles.direct_contacts {
        let endpoint = format!("share://direct/{}", contact.id);
        choices.push(TargetChoice {
            target: crate::share::PeerOpenTarget::Direct {
                contact_id: contact.id.clone(),
            },
            endpoint: endpoint.clone(),
            label: contact.display_name.clone(),
            selectors: vec![
                endpoint,
                contact.id.clone(),
                contact.display_name.clone(),
                contact.remote_device_id.clone().unwrap_or_default(),
                contact.expected_fingerprint.clone(),
            ],
            ready: contact
                .presence
                .as_ref()
                .is_some_and(|presence| presence.is_current_at(now))
                && contact.access_state == crate::share::DirectAccessState::Accepted,
        });
    }
    for room in &profiles.rooms {
        for member in &room.members {
            let endpoint = format!("share://room/{}/{}", room.room_id, member.device_id);
            choices.push(TargetChoice {
                target: crate::share::PeerOpenTarget::RoomDevice {
                    room_id: room.room_id.clone(),
                    device_id: member.device_id.clone(),
                },
                endpoint: endpoint.clone(),
                label: format!("{} / {}", room.name, member.device_name),
                selectors: vec![
                    endpoint,
                    member.device_id.clone(),
                    member.device_name.clone(),
                    member.fingerprint.clone(),
                    format!("{}/{}", room.name, member.device_name),
                ],
                ready: member
                    .presence
                    .as_ref()
                    .is_some_and(|presence| presence.is_current_at(now))
                    && !member.blocked,
            });
        }
    }
    choices
}

fn format_choices(choices: &[impl std::borrow::Borrow<TargetChoice>], heading: &str) -> String {
    if choices.is_empty() {
        return format!(
            "{heading}. Add a peer with `se connections add-peer --code ...`, then inspect `se connections list`"
        );
    }
    let rows = choices
        .iter()
        .map(|choice| {
            let choice = choice.borrow();
            format!(
                "  {}\t{}\t{}\n    se exec {} -- PROGRAM ...",
                choice.endpoint,
                choice.label,
                if choice.ready { "ready" } else { "offline" },
                choice.endpoint,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{heading}:\n{rows}")
}

fn selector_matches(selector: &str, candidate: &str) -> bool {
    !candidate.is_empty()
        && (candidate.eq_ignore_ascii_case(selector)
            || (selector.len() >= 4
                && candidate
                    .to_ascii_lowercase()
                    .starts_with(&selector.to_ascii_lowercase())))
}

#[cfg(test)]
mod tests {
    use super::{exit_code, parse_environment, selector_matches};

    #[test]
    fn environment_and_selector_inputs_are_literal() {
        let env = parse_environment(vec!["A=space $ ' quote".into(), "EMPTY=".into()]).unwrap();
        assert_eq!(env["A"], "space $ ' quote");
        assert_eq!(env["EMPTY"], "");
        assert!(selector_matches("abcd", "abcdef"));
        assert!(!selector_matches("abc", "abcdef"));
    }

    #[test]
    fn terminal_status_maps_to_documented_cli_codes() {
        let mut terminal = crate::share::ExecTerminal {
            exec_id: crate::share::ExecId::parse("00".repeat(16)).unwrap(),
            kind: crate::share::ExecTerminalKind::Exited,
            exit_code: Some(7),
            signal: None,
            message: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            output_truncated: false,
        };
        assert_eq!(exit_code(&terminal), 7);
        terminal.kind = crate::share::ExecTerminalKind::TimedOut;
        assert_eq!(exit_code(&terminal), 124);
        terminal.kind = crate::share::ExecTerminalKind::Cancelled;
        assert_eq!(exit_code(&terminal), 130);
        terminal.kind = crate::share::ExecTerminalKind::Revoked;
        assert_eq!(exit_code(&terminal), 125);
    }
}
