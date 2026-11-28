//! The hidden helper `se elevated exec|confirm|run` (`core/helper_args.rs`).
//!
//! - `exec` runs elevated: it checks its channel to the caller (Windows: the
//!   pipes and their creator, the program's SHA-256), sends the ready frame,
//!   starts the command directly and relays its streams as frames until it
//!   ended. When the caller's frames end without an end-of-input frame, the
//!   caller is gone and the command is cancelled.
//! - `confirm` only exits 0 when it runs elevated.
//! - `run` asks for the elevation on this device and passes its own standard
//!   streams through (the Share host wraps an elevated start into it).
use std::io::{Read, Write};

use super::helper_args::{self, HelperCall};
use super::types::{ElevateError, ElevatedEvent, ElevatedExit, Elevation};
use super::wire::{self, FromHelper, ToHelper};

/// Exit status of `run` when the user declined the dialog.
pub const DECLINED_EXIT: i32 = 126;
/// Exit status of `run` when the elevation could not happen otherwise.
pub const UNAVAILABLE_EXIT: i32 = 125;

/// Runs `se elevated <args>`; the exit status of the process.
pub fn run_helper(args: &[String]) -> Result<i32, String> {
    match args.split_first() {
        Some((verb, rest)) if verb == "exec" => exec(rest),
        Some((verb, rest)) if verb == "confirm" => confirm(rest),
        Some((verb, rest)) if verb == "run" => run_client(rest),
        _ => Err("se elevated is an internal helper of Smart Explorer (exec, confirm, run)".into()),
    }
}

fn exec(args: &[String]) -> Result<i32, String> {
    let call = helper_args::parse(args)?;
    if !super::platform::is_elevated() {
        return Err("se elevated exec runs only with administrator or root rights".into());
    }
    let (reader, mut writer) = super::platform::helper_transport(&call)?;
    wire::send_from_helper(&mut *writer, &FromHelper::Ready)
        .map_err(|error| format!("se elevated: the caller is gone: {error}"))?;
    let mut child = match super::direct::spawn(&call.spec, Elevation::Already) {
        Ok(child) => child,
        Err(error) => {
            let _ = wire::send_from_helper(&mut *writer, &FromHelper::Failed(error.to_string()));
            return Ok(1);
        }
    };
    let input = child.input();
    let feeder = std::thread::Builder::new()
        .name("se-elevated-input".into())
        .spawn(move || feed(reader, &input))
        .map_err(|error| format!("se elevated: input thread: {error}"));
    if let Err(error) = feeder {
        child.input().cancel();
        return Err(error);
    }
    let mut caller_gone = false;
    while let Some(event) = child.next_event() {
        let frame = match event {
            ElevatedEvent::Stdout(bytes) => FromHelper::Stdout(bytes),
            ElevatedEvent::Stderr(bytes) => FromHelper::Stderr(bytes),
            ElevatedEvent::Exited(exit) => FromHelper::Exit(exit),
        };
        if caller_gone {
            continue;
        }
        if wire::send_from_helper(&mut *writer, &frame).is_err() {
            caller_gone = true;
            child.input().cancel();
        }
    }
    Ok(0)
}

/// Passes the caller's frames to the command; a broken or ended channel
/// without an end-of-input frame cancels it.
fn feed(mut reader: Box<dyn Read + Send>, input: &super::ElevatedInput) {
    loop {
        match wire::read_to_helper(&mut *reader) {
            Ok(Some(ToHelper::Stdin(bytes))) => {
                // A command that closed its input ignores further input.
                let _ = input.stdin(&bytes);
            }
            Ok(Some(ToHelper::Eof)) => {
                let _ = input.eof();
            }
            Ok(Some(ToHelper::Cancel)) | Ok(None) | Err(_) => {
                input.cancel();
                return;
            }
        }
    }
}

fn confirm(args: &[String]) -> Result<i32, String> {
    if !matches!(args.first().map(String::as_str), Some("--reason")) {
        return Err("se elevated confirm needs --reason TEXT".into());
    }
    if super::platform::is_elevated() {
        Ok(0)
    } else {
        Err("se elevated confirm runs only with administrator or root rights".into())
    }
}

fn run_client(args: &[String]) -> Result<i32, String> {
    let call: HelperCall = helper_args::parse(args)?;
    let mut child = match super::spawn_elevated(call.spec) {
        Ok(child) => child,
        Err(error) => {
            eprintln!("se: {error}");
            return Ok(match error {
                ElevateError::Declined => DECLINED_EXIT,
                _ => UNAVAILABLE_EXIT,
            });
        }
    };
    let input = child.input();
    let _ = std::thread::Builder::new()
        .name("se-elevated-stdin".into())
        .spawn(move || copy_stdin(&input));
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    while let Some(event) = child.next_event() {
        match event {
            ElevatedEvent::Stdout(bytes) => {
                let _ = stdout.write_all(&bytes).and_then(|()| stdout.flush());
            }
            ElevatedEvent::Stderr(bytes) => {
                let _ = stderr.write_all(&bytes).and_then(|()| stderr.flush());
            }
            ElevatedEvent::Exited(exit) => return Ok(run_status(&exit)),
        }
    }
    Ok(UNAVAILABLE_EXIT)
}

fn run_status(exit: &ElevatedExit) -> i32 {
    if let Some(error) = &exit.error {
        eprintln!("se: {error}");
    }
    match exit.status() {
        Some(status) => status,
        None if exit.cancelled => 130,
        None => UNAVAILABLE_EXIT,
    }
}

fn copy_stdin(input: &super::ElevatedInput) {
    let mut stdin = std::io::stdin();
    let mut buffer = vec![0u8; 16 * 1024];
    loop {
        match stdin.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if input.stdin(&buffer[..read]).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = input.eof();
}
