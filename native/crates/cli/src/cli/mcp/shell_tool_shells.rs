//! The shell of the `shell` tool on this device (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, M6), chosen as on Share
//! peers (`share/core/exec_shell.rs`):
//!
//! - Linux: the login shell named by an absolute `$SHELL`, else `/bin/sh`;
//!   `command` runs as `<shell> -c <command>`. `powershell` and `cmd` are
//!   refused.
//! - Windows: PowerShell 7 (`pwsh.exe` on the `PATH` or in
//!   `%ProgramFiles%\PowerShell\7`), else Windows PowerShell 5.1, with the
//!   one-shot script of the Share host (parse errors with line and column,
//!   the same exit status); `cmd` runs `cmd.exe /D /S /C`, `sh` an `sh.exe`
//!   on the `PATH` (Git for Windows, MSYS2) when there is one.
//!
//! A persistent session runs the same shell reading commands from its input:
//! a POSIX shell (`bash`, `sh`, `dash`, `ksh`, `zsh` as `$SHELL`, else
//! `/bin/sh`) or PowerShell with `-Command -`.
//!
//! The facts about the device come in as [`LocalHost`]; the choice itself is
//! platform-free and tested for both systems.
use std::path::{Path, PathBuf};

use crate::cli::shell_session::Dialect;
use crate::elevate::ElevatedCommand;
use crate::share::exec_shell::{
    powershell_one_shot, POWERSHELL_ARGS, PWSH_LABEL, WINDOWS_POWERSHELL_LABEL,
};
use crate::share::{ExecCommand, ExecShell};

/// What this device offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LocalHost {
    Windows {
        /// `pwsh.exe` of PowerShell 7, when installed.
        pwsh: Option<String>,
        /// `powershell.exe` of Windows PowerShell 5.1.
        windows_powershell: String,
        /// An `sh.exe` on the `PATH`.
        sh: Option<String>,
    },
    Posix {
        /// The login shell (absolute `$SHELL` or `/bin/sh`).
        shell: String,
    },
}

impl LocalHost {
    /// The facts of the running system.
    pub(super) fn detect() -> Self {
        if cfg!(windows) {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let on_path = |name: &str| {
                std::env::split_paths(&path)
                    .filter(|directory| directory.is_absolute())
                    .map(|directory| directory.join(name))
                    .find(|candidate| present(candidate))
            };
            let installed = ["ProgramFiles", "ProgramW6432"]
                .iter()
                .filter_map(std::env::var_os)
                .map(|root| PathBuf::from(root).join(r"PowerShell\7\pwsh.exe"))
                .find(|candidate| present(candidate));
            let system = std::env::var_os("SystemRoot")
                .or_else(|| std::env::var_os("windir"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
            Self::Windows {
                pwsh: on_path("pwsh.exe").or(installed).map(text),
                windows_powershell: text(
                    system.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
                ),
                sh: on_path("sh.exe").map(text),
            }
        } else {
            let shell = std::env::var("SHELL")
                .ok()
                .filter(|shell| shell.starts_with('/') && Path::new(shell).is_file())
                .unwrap_or_else(|| "/bin/sh".to_string());
            Self::Posix { shell }
        }
    }
}

/// A file, or an app execution alias (a reparse point `metadata` cannot
/// follow, which process creation starts), as the Share host checks it.
fn present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_dir())
}

fn text(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

/// The program that runs `command` and the result name of its shell
/// (`none` for argv).
pub(super) fn one_shot(
    host: &LocalHost,
    choice: Option<ExecShell>,
    command: &ExecCommand,
) -> Result<(ElevatedCommand, String), String> {
    let line = match command {
        ExecCommand::Argv { program, args } => {
            return Ok((
                ElevatedCommand::Argv {
                    program: program.clone(),
                    args: args.clone(),
                },
                crate::cli::exec_peer::NO_SHELL.to_string(),
            ))
        }
        ExecCommand::Shell { command } => command,
        ExecCommand::Login => return Err("an interactive login shell needs a terminal".into()),
    };
    match host {
        LocalHost::Posix { shell } => match choice {
            None | Some(ExecShell::Standard | ExecShell::Sh) => Ok((
                ElevatedCommand::Argv {
                    program: shell.clone(),
                    args: vec!["-c".into(), line.clone()],
                },
                file_name(shell),
            )),
            Some(other) => Err(format!(
                "this device runs Linux; its commands run in the login shell, and `{}` is only for Windows: omit `shell` or use sh",
                other.as_str()
            )),
        },
        LocalHost::Windows {
            pwsh,
            windows_powershell,
            sh,
        } => match choice {
            None | Some(ExecShell::Standard | ExecShell::PowerShell) => {
                let (program, label) = match pwsh {
                    Some(pwsh) => (pwsh.clone(), PWSH_LABEL),
                    None => (windows_powershell.clone(), WINDOWS_POWERSHELL_LABEL),
                };
                let mut args: Vec<String> = POWERSHELL_ARGS.iter().map(|o| o.to_string()).collect();
                args.push(powershell_one_shot(line));
                Ok((ElevatedCommand::Argv { program, args }, label.to_string()))
            }
            Some(ExecShell::Cmd) => Ok((ElevatedCommand::CmdLine(line.clone()), "cmd".into())),
            Some(ExecShell::Sh) => match sh {
                Some(sh) => Ok((
                    ElevatedCommand::Argv {
                        program: sh.clone(),
                        args: vec!["-c".into(), line.clone()],
                    },
                    "sh".into(),
                )),
                None => Err(
                    "this Windows device has no sh.exe on its PATH (Git for Windows or MSYS2 bring one); use powershell (the default) or cmd".into(),
                ),
            },
        },
    }
}

/// The shell of a persistent session on this device and its dialect.
pub(super) fn session_shell(
    host: &LocalHost,
    choice: Option<ExecShell>,
) -> Result<(ElevatedCommand, Dialect), String> {
    match host {
        LocalHost::Posix { shell } => match choice {
            None | Some(ExecShell::Standard | ExecShell::Sh) => {
                let posix = matches!(
                    file_name(shell).as_str(),
                    "bash" | "sh" | "dash" | "ksh" | "zsh"
                );
                let program = if posix {
                    shell.clone()
                } else {
                    "/bin/sh".into()
                };
                Ok((
                    ElevatedCommand::Argv {
                        program,
                        args: Vec::new(),
                    },
                    Dialect::Posix,
                ))
            }
            Some(other) => Err(format!(
                "sessions on this Linux device run in a POSIX shell; `{}` is only for Windows",
                other.as_str()
            )),
        },
        LocalHost::Windows {
            pwsh,
            windows_powershell,
            sh,
        } => match choice {
            None | Some(ExecShell::Standard | ExecShell::PowerShell) => Ok((
                ElevatedCommand::Argv {
                    program: pwsh.clone().unwrap_or_else(|| windows_powershell.clone()),
                    args: ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "-"]
                        .map(String::from)
                        .to_vec(),
                },
                Dialect::PowerShell,
            )),
            Some(ExecShell::Sh) => match sh {
                Some(sh) => Ok((
                    ElevatedCommand::Argv {
                        program: sh.clone(),
                        args: Vec::new(),
                    },
                    Dialect::Posix,
                )),
                None => Err("this Windows device has no sh.exe on its PATH".into()),
            },
            Some(ExecShell::Cmd) => Err(
                "sessions on Windows run in PowerShell; `cmd` works only without `session`".into(),
            ),
        },
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("sh")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(pwsh: bool, sh: bool) -> LocalHost {
        LocalHost::Windows {
            pwsh: pwsh.then(|| r"C:\Program Files\PowerShell\7\pwsh.exe".to_string()),
            windows_powershell: r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into(),
            sh: sh.then(|| r"C:\Program Files\Git\usr\bin\sh.exe".to_string()),
        }
    }

    fn shell(command: &str) -> ExecCommand {
        ExecCommand::Shell {
            command: command.into(),
        }
    }

    #[test]
    fn shell_elevate_task_local_shell_follows_the_share_host_rules() {
        let linux = LocalHost::Posix {
            shell: "/usr/bin/bash".into(),
        };
        let (command, label) = one_shot(&linux, None, &shell("echo $HOME")).unwrap();
        assert_eq!(label, "bash");
        assert_eq!(
            command,
            ElevatedCommand::Argv {
                program: "/usr/bin/bash".into(),
                args: vec!["-c".into(), "echo $HOME".into()],
            }
        );
        assert!(one_shot(&linux, Some(ExecShell::PowerShell), &shell("x")).is_err());
        let argv = ExecCommand::Argv {
            program: "git".into(),
            args: vec!["status".into()],
        };
        assert_eq!(one_shot(&linux, None, &argv).unwrap().1, "none");

        let (command, label) = one_shot(&windows(true, false), None, &shell("dir")).unwrap();
        assert_eq!(label, PWSH_LABEL);
        let ElevatedCommand::Argv { program, args } = command else {
            panic!("argv");
        };
        assert!(program.ends_with("pwsh.exe"));
        assert_eq!(args[..6], POWERSHELL_ARGS.map(String::from));
        let (_, label) = one_shot(&windows(false, false), None, &shell("dir")).unwrap();
        assert_eq!(label, WINDOWS_POWERSHELL_LABEL);
        assert_eq!(
            one_shot(
                &windows(false, false),
                Some(ExecShell::Cmd),
                &shell("dir /b")
            )
            .unwrap(),
            (ElevatedCommand::CmdLine("dir /b".into()), "cmd".to_string())
        );
        assert!(one_shot(&windows(false, false), Some(ExecShell::Sh), &shell("ls")).is_err());
        assert_eq!(
            one_shot(&windows(false, true), Some(ExecShell::Sh), &shell("ls"))
                .unwrap()
                .1,
            "sh"
        );
    }

    #[test]
    fn shell_elevate_task_local_sessions_read_commands_from_their_input() {
        let fish = LocalHost::Posix {
            shell: "/usr/bin/fish".into(),
        };
        assert_eq!(
            session_shell(&fish, None).unwrap(),
            (
                ElevatedCommand::Argv {
                    program: "/bin/sh".into(),
                    args: Vec::new()
                },
                Dialect::Posix
            )
        );
        let (command, dialect) = session_shell(&windows(false, false), None).unwrap();
        assert_eq!(dialect, Dialect::PowerShell);
        let ElevatedCommand::Argv { args, .. } = command else {
            panic!("argv");
        };
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(session_shell(&windows(true, false), Some(ExecShell::Cmd)).is_err());
    }
}
