//! `se exec` (`exec.rs`): literal inputs, exit codes and the shell choice.
use super::{exit_code, parse_environment, selector_matches, shell_choice};

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
        shell: None,
    };
    assert_eq!(exit_code(&terminal), 7);
    terminal.kind = crate::share::ExecTerminalKind::TimedOut;
    assert_eq!(exit_code(&terminal), 124);
    terminal.kind = crate::share::ExecTerminalKind::Cancelled;
    assert_eq!(exit_code(&terminal), 130);
    terminal.kind = crate::share::ExecTerminalKind::Revoked;
    assert_eq!(exit_code(&terminal), 125);
}

#[test]
fn exec_shell_task_se_exec_sends_the_shell_choice() {
    use crate::share::ExecShell;
    // `--shell` alone: the peer's own shell (PowerShell on Windows peers).
    assert_eq!(shell_choice(None, true), Ok(Some(ExecShell::Standard)));
    // argv has no shell.
    assert_eq!(shell_choice(None, false), Ok(None));
    for (kind, shell) in [
        ("powershell", ExecShell::PowerShell),
        ("cmd", ExecShell::Cmd),
        ("sh", ExecShell::Sh),
    ] {
        assert_eq!(shell_choice(Some(kind), true), Ok(Some(shell)));
    }
    assert!(shell_choice(Some("bash"), true).is_err());
}
