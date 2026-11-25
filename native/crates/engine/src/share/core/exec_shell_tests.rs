//! The shell of Share Exec shell commands (`exec_shell.rs`).
use std::collections::BTreeMap;
use std::io;

use super::*;
use crate::share::exec_types::{ExecCommand, ExecId, ExecStart};

fn start(command: ExecCommand, shell: Option<ExecShell>) -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse("00".repeat(16)).unwrap(),
        command,
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
        shell,
    }
}

fn shell_command(text: &str, shell: Option<ExecShell>) -> ExecStart {
    start(
        ExecCommand::Shell {
            command: text.into(),
        },
        shell,
    )
}

fn argv() -> ExecCommand {
    ExecCommand::Argv {
        program: "git".into(),
        args: vec!["status".into()],
    }
}

fn pwsh() -> (String, &'static str) {
    (
        r"C:\Program Files\PowerShell\7\pwsh.exe".to_string(),
        PWSH_LABEL,
    )
}

#[test]
fn exec_shell_task_choice_is_an_optional_field_of_shell_commands() {
    assert_eq!(ExecShell::parse("PowerShell"), Ok(ExecShell::PowerShell));
    assert_eq!(ExecShell::parse("pwsh"), Ok(ExecShell::PowerShell));
    assert_eq!(ExecShell::parse("cmd"), Ok(ExecShell::Cmd));
    assert_eq!(ExecShell::parse(" sh "), Ok(ExecShell::Sh));
    assert!(ExecShell::parse("bash").is_err());
    for (name, shell) in [
        ("standard", ExecShell::Standard),
        ("powershell", ExecShell::PowerShell),
        ("cmd", ExecShell::Cmd),
        ("sh", ExecShell::Sh),
    ] {
        assert_eq!(serde_json::to_value(shell).unwrap(), name);
        assert_eq!(shell.as_str(), name);
    }
    // Without a choice a start keeps the bytes and digest of earlier clients.
    let plain = shell_command("dir", None);
    let encoded = serde_json::to_string(&plain).unwrap();
    assert!(!encoded.contains("\"shell\":"), "{encoded}");
    let chosen = shell_command("dir", Some(ExecShell::PowerShell));
    let encoded = serde_json::to_string(&chosen).unwrap();
    assert!(encoded.ends_with(",\"shell\":\"powershell\"}"), "{encoded}");
    assert_eq!(serde_json::from_str::<ExecStart>(&encoded).unwrap(), chosen);
    assert_ne!(chosen.digest().unwrap(), plain.digest().unwrap());
    chosen.validate().unwrap();
    // Only a shell command has a shell.
    let error = start(argv(), Some(ExecShell::Cmd)).validate().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    start(argv(), None).validate().unwrap();
}

#[test]
fn exec_shell_task_an_older_host_gets_the_start_without_the_choice() {
    // A host without the feature would drop the field and then refuse the
    // digest; it gets the start of an earlier client and runs its own shell.
    let chosen = shell_command("dir", Some(ExecShell::Standard));
    let older = for_host(chosen.clone(), &["exec_terminal_v1".to_string()]);
    assert_eq!(older.shell, None);
    assert_eq!(
        older.digest().unwrap(),
        shell_command("dir", None).digest().unwrap()
    );
    let current = for_host(
        chosen.clone(),
        &[
            "exec_terminal_v1".to_string(),
            EXEC_SHELL_FEATURE.to_string(),
        ],
    );
    assert_eq!(current, chosen);
    assert_eq!(
        for_host(shell_command("dir", None), &[]),
        shell_command("dir", None)
    );
    // Such a host is named by its provider and the shell it always ran.
    for (provider, family, legacy) in [
        ("Windows Job Object", HostFamily::Windows, "cmd"),
        ("systemd transient cgroup", HostFamily::Linux, "login shell"),
        ("android-subreaper", HostFamily::Android, "sh"),
    ] {
        assert_eq!(HostFamily::of_provider(provider), Some(family));
        assert_eq!(family.legacy_shell(), legacy);
    }
    assert_eq!(HostFamily::of_provider("unknown provider"), None);
    assert_eq!(HostFamily::Windows.label(), "windows");
}

#[test]
fn exec_shell_task_windows_runs_powershell_by_default_and_cmd_on_request() {
    let mut standard = shell_command("cd C:\\ && dir", Some(ExecShell::Standard));
    assert_eq!(
        windows(&mut standard, pwsh).unwrap().as_deref(),
        Some("pwsh 7")
    );
    assert_eq!(standard.shell, None);
    let ExecCommand::Argv { program, args } = &standard.command else {
        panic!("PowerShell runs as an argv start: {:?}", standard.command);
    };
    assert_eq!(program, r"C:\Program Files\PowerShell\7\pwsh.exe");
    assert_eq!(args.len(), POWERSHELL_ARGS.len() + 1);
    assert_eq!(&args[..POWERSHELL_ARGS.len()], &POWERSHELL_ARGS[..]);
    assert_eq!(
        args[POWERSHELL_ARGS.len()],
        powershell_one_shot("cd C:\\ && dir")
    );
    standard.validate().unwrap();
    // Windows PowerShell 5.1 when PowerShell 7 is missing.
    let mut explicit = shell_command("Get-Date", Some(ExecShell::PowerShell));
    let windows_powershell = || {
        (
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".to_string(),
            WINDOWS_POWERSHELL_LABEL,
        )
    };
    assert_eq!(
        windows(&mut explicit, windows_powershell)
            .unwrap()
            .as_deref(),
        Some("powershell 5.1")
    );
    // cmd on request, and for starts without a choice (older clients, the
    // start line of se shell sessions): the supervisor runs cmd.exe.
    for shell in [None, Some(ExecShell::Cmd)] {
        let mut cmd = shell_command("echo %OS%", shell);
        let before = cmd.clone();
        let label = windows(&mut cmd, || -> (String, &'static str) {
            unreachable!("cmd needs no PowerShell")
        });
        assert_eq!(label.unwrap().as_deref(), Some("cmd"));
        assert_eq!(cmd, before);
    }
    let error = windows(&mut shell_command("ls", Some(ExecShell::Sh)), pwsh).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("runs Windows"), "{error}");
    assert_eq!(windows(&mut start(argv(), None), pwsh).unwrap(), None);
    // A command beyond the Windows command line is refused with advice.
    let mut long = shell_command(&"x".repeat(30_000), Some(ExecShell::Standard));
    let error = windows(&mut long, pwsh).unwrap_err();
    assert!(error.to_string().contains("script file"), "{error}");
}

#[test]
fn exec_shell_task_linux_and_android_keep_the_login_shell() {
    let mut environment = BTreeMap::new();
    environment.insert("SHELL".to_string(), "/usr/bin/zsh".to_string());
    let named = |request: &ExecStart, family, environment: &BTreeMap<String, String>| {
        posix(request, family, environment).unwrap()
    };
    let plain = shell_command("ls && pwd", None);
    assert_eq!(
        named(&plain, HostFamily::Linux, &environment).as_deref(),
        Some("zsh")
    );
    let standard = shell_command("ls", Some(ExecShell::Standard));
    assert_eq!(
        named(&standard, HostFamily::Linux, &BTreeMap::new()).as_deref(),
        Some("sh")
    );
    // A relative SHELL is not honoured, like the supervisor does.
    environment.insert("SHELL".to_string(), "bash".to_string());
    let sh = shell_command("ls", Some(ExecShell::Sh));
    assert_eq!(
        named(&sh, HostFamily::Linux, &environment).as_deref(),
        Some("sh")
    );
    assert_eq!(
        named(&plain, HostFamily::Android, &environment).as_deref(),
        Some("sh")
    );
    for shell in [ExecShell::PowerShell, ExecShell::Cmd] {
        let request = shell_command("dir", Some(shell));
        let error = posix(&request, HostFamily::Linux, &environment).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("runs linux"), "{error}");
    }
    assert_eq!(
        named(&start(argv(), None), HostFamily::Android, &environment),
        None
    );
}

#[test]
fn exec_shell_task_powershell_reports_parse_errors_itself() {
    let statement = powershell_statement("cd C:\\ && dir", "", None);
    // One line without the command's own text or quoting characters.
    assert!(!statement.contains('\n'), "{statement}");
    assert!(!statement.contains('"'), "{statement}");
    assert!(!statement.contains("&& dir"), "{statement}");
    assert!(statement.contains(&powershell_text("cd C:\\ && dir")));
    // The command is parsed before it runs; a parse error is written as
    // PowerShell words it, not as the exception of ScriptBlock.Create.
    assert!(statement
        .contains("[System.Management.Automation.Language.Parser]::ParseInput($global:__se_c,"));
    assert!(statement.contains("'se: PowerShell cannot parse line '"));
    assert!(statement.contains("+$__se_x.Message+"));
    assert!(statement.contains("Windows PowerShell 5.1 has no && and ||"));
    assert!(statement.contains(". ([ScriptBlock]::Create($global:__se_c))"));
    assert!(
        statement.ends_with("elseif($null -ne $global:LASTEXITCODE){$global:LASTEXITCODE}else{0}")
    );
    // A prelude of se runs first; only the command is parsed for errors.
    let placed = powershell_statement("Get-Date", "Set-Location x\n", Some("data"));
    assert!(placed.contains(&format!(
        "{} | . ([ScriptBlock]::Create({}+$global:__se_c))",
        powershell_text("data"),
        powershell_text("Set-Location x\n")
    )));
    let one_shot = powershell_one_shot("Get-Date");
    assert!(one_shot
        .starts_with("try{[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)}catch{};"));
    assert!(one_shot.contains(&powershell_statement("Get-Date", "", None)));
    assert!(one_shot.ends_with(";exit $global:__se_s"));
    assert_eq!(
        powershell_text("ä"),
        "[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('w6Q='))"
    );
}
