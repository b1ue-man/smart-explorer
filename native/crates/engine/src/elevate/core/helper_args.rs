//! The arguments of the hidden helper `se elevated …`. The command, its
//! working directory and environment travel as plain arguments, so the
//! system dialog that names the started program (polkit shows the whole
//! command line, UAC under „Details“) shows what will run, and the helper
//! runs exactly that:
//!
//! - `exec [--pipe NAME --server-pid PID] [--image-hash HEX] [--cwd DIR]
//!   [--env NAME=VALUE]… (--cmd-line LINE | -- PROGRAM [ARG]…)` runs the
//!   command elevated and talks the frames of `wire.rs` (Windows: over the
//!   pipes `NAME-in`/`NAME-out` of the process `PID`; Linux: over standard
//!   input and output).
//! - `confirm --reason TEXT` only proves that the system dialog was
//!   answered with yes (exit 0).
//! - `run [--cwd DIR] [--env NAME=VALUE]… (--cmd-line LINE | -- PROGRAM
//!   [ARG]…)` is the client side for a Share host: it asks for the
//!   elevation on this device and passes its own standard streams through.
use std::collections::BTreeMap;

use super::types::{ElevatedCommand, ElevatedSpec};

/// The Windows pipes of an `exec` helper and who created them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeBinding {
    /// `\\.\pipe\se-elevate-<nonce>`; the helper reads `-in` and writes `-out`.
    pub name: String,
    /// Process that created the pipes (checked by the helper).
    pub server_pid: u32,
}

/// A parsed `exec` or `run` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperCall {
    pub spec: ElevatedSpec,
    pub pipe: Option<PipeBinding>,
    /// SHA-256 of the se program the caller locked and expects to run.
    pub image_hash: Option<String>,
}

/// The arguments after `se elevated` for the verb `exec` or `run`.
pub fn build(verb: &str, call: &HelperCall) -> Vec<String> {
    let mut args = vec![verb.to_string()];
    if let Some(pipe) = &call.pipe {
        args.extend([
            "--pipe".into(),
            pipe.name.clone(),
            "--server-pid".into(),
            pipe.server_pid.to_string(),
        ]);
    }
    if let Some(hash) = &call.image_hash {
        args.extend(["--image-hash".into(), hash.clone()]);
    }
    if let Some(cwd) = &call.spec.cwd {
        args.extend(["--cwd".into(), cwd.clone()]);
    }
    for (name, value) in &call.spec.env {
        args.extend(["--env".into(), format!("{name}={value}")]);
    }
    match &call.spec.command {
        ElevatedCommand::CmdLine(line) => args.extend(["--cmd-line".into(), line.clone()]),
        ElevatedCommand::Argv {
            program,
            args: rest,
        } => {
            args.push("--".into());
            args.push(program.clone());
            args.extend(rest.iter().cloned());
        }
    }
    args
}

/// Parses the arguments after the verb (`exec`/`run`). A `--` before the
/// program is optional: the first word that is no option of the helper
/// starts the program, so an argument parser that consumed `--` changes
/// nothing.
pub fn parse(args: &[String]) -> Result<HelperCall, String> {
    let mut pipe_name = None;
    let mut server_pid = None;
    let mut image_hash = None;
    let mut cwd = None;
    let mut env = BTreeMap::new();
    let mut cmd_line = None;
    let mut index = 0;
    let value = |index: usize, option: &str| -> Result<String, String> {
        args.get(index + 1)
            .cloned()
            .ok_or_else(|| format!("se elevated: {option} needs a value"))
    };
    while index < args.len() {
        let word = args[index].as_str();
        match word {
            "--" => {
                index += 1;
                break;
            }
            "--pipe" => pipe_name = Some(value(index, word)?),
            "--server-pid" => {
                let pid = value(index, word)?;
                server_pid = Some(
                    pid.parse::<u32>()
                        .map_err(|_| format!("se elevated: invalid --server-pid {pid}"))?,
                );
            }
            "--image-hash" => {
                let hash = value(index, word)?;
                if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err("se elevated: --image-hash needs 64 hexadecimal digits".into());
                }
                image_hash = Some(hash.to_ascii_lowercase());
            }
            "--cwd" => cwd = Some(value(index, word)?),
            "--env" => {
                let pair = value(index, word)?;
                let (name, value) = pair
                    .split_once('=')
                    .filter(|(name, _)| !name.is_empty())
                    .ok_or_else(|| format!("se elevated: --env needs NAME=VALUE, got {pair:?}"))?;
                env.insert(name.to_string(), value.to_string());
            }
            "--cmd-line" => cmd_line = Some(value(index, word)?),
            _ => break,
        }
        index += 2;
    }
    let rest = &args[index.min(args.len())..];
    let command = match (cmd_line, rest.split_first()) {
        (Some(_), Some(_)) => {
            return Err("se elevated: give either --cmd-line or a program, not both".into())
        }
        (Some(line), None) => ElevatedCommand::CmdLine(line),
        (None, Some((program, rest))) if !program.is_empty() => ElevatedCommand::Argv {
            program: program.clone(),
            args: rest.to_vec(),
        },
        _ => return Err("se elevated: the program to run is missing".into()),
    };
    let pipe = match (pipe_name, server_pid) {
        (Some(name), Some(server_pid)) => Some(PipeBinding { name, server_pid }),
        (None, None) => None,
        _ => return Err("se elevated: --pipe and --server-pid belong together".into()),
    };
    Ok(HelperCall {
        spec: ElevatedSpec { command, cwd, env },
        pipe,
        image_hash,
    })
}

/// One Windows command line of `args` as `CommandLineToArgvW` and the C
/// runtime split it again (quotes around words with blanks or quotes,
/// backslashes doubled before a quote).
pub fn windows_command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| windows_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn windows_quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return arg.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.extend(std::iter::repeat('\\').take(backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat('\\').take(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn shell_elevate_task_helper_arguments_round_trip() {
        let mut env = BTreeMap::new();
        env.insert("MODE".to_string(), "a=b c".to_string());
        let call = HelperCall {
            spec: ElevatedSpec {
                command: ElevatedCommand::Argv {
                    program: "/bin/sh".into(),
                    args: strings(&["-c", "--cwd x", "--"]),
                },
                cwd: Some("/srv/my app".into()),
                env,
            },
            pipe: Some(PipeBinding {
                name: r"\\.\pipe\se-elevate-00ff".into(),
                server_pid: 4711,
            }),
            image_hash: Some("ab".repeat(32)),
        };
        let built = build("exec", &call);
        assert_eq!(built[0], "exec");
        assert_eq!(parse(&built[1..]).unwrap(), call);
        // An argument parser that consumed the `--`.
        let without: Vec<String> = built[1..].iter().filter(|w| *w != "--").cloned().collect();
        let mut expected = call.clone();
        if let ElevatedCommand::Argv { args, .. } = &mut expected.spec.command {
            args.pop();
        }
        assert_eq!(parse(&without).unwrap(), expected);
        let cmd = HelperCall {
            spec: ElevatedSpec {
                command: ElevatedCommand::CmdLine("echo \"%PATH%\"".into()),
                cwd: None,
                env: BTreeMap::new(),
            },
            pipe: None,
            image_hash: None,
        };
        assert_eq!(parse(&build("run", &cmd)[1..]).unwrap(), cmd);
    }

    #[test]
    fn shell_elevate_task_helper_arguments_refuse_incomplete_calls() {
        for broken in [
            strings(&[]),
            strings(&["--cwd"]),
            strings(&["--env", "=x", "--", "true"]),
            strings(&["--pipe", "p", "--", "true"]),
            strings(&["--server-pid", "x", "--", "true"]),
            strings(&["--image-hash", "12", "--", "true"]),
            strings(&["--cmd-line", "dir", "--", "true"]),
        ] {
            assert!(parse(&broken).is_err(), "{broken:?}");
        }
    }

    #[test]
    fn shell_elevate_task_windows_command_line_survives_argv_splitting() {
        let line = windows_command_line(&strings(&[
            "plain",
            "with space",
            "",
            "quote\"inside",
            r"C:\dir\",
            r#"back\\"slash"#,
        ]));
        assert_eq!(
            line,
            r#"plain "with space" "" "quote\"inside" C:\dir\ "back\\\\\"slash""#
        );
    }
}
