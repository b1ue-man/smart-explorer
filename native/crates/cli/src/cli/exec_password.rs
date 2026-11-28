//! The password dialog for `sudo` on SSH remotes (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C „Erhöhung
//! entfernt“): never in the chat, never stored. Windows asks in the system's
//! credential dialog (CredUI, `crate::elevate::ask_password`); Linux in a
//! small window of se (`se elevated ask-password`, a process of its own,
//! because a window needs the main thread), which hands the password to
//! this process over a pipe and exits.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use eframe::egui;
use zeroize::Zeroizing;

/// Exit status of the window when the user cancelled.
const CANCELLED: i32 = 1;

/// Asks the user for a password; `Ok(None)` when they cancelled.
pub(in crate::cli) fn ask(
    caption: &str,
    message: &str,
) -> Result<Option<Zeroizing<String>>, String> {
    if cfg!(windows) {
        return crate::elevate::ask_password(caption, message).map_err(|error| error.to_string());
    }
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(
            "no desktop session for the password dialog (neither DISPLAY nor WAYLAND_DISPLAY is set); allow the command for this account without a password (sudoers NOPASSWD) or run it in a terminal".into(),
        );
    }
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(program)
        .args([
            "elevated",
            "ask-password",
            "--caption",
            caption,
            "--message",
            message,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot open the password dialog: {error}"))?;
    let mut answer = Zeroizing::new(Vec::new());
    if let Some(mut stdout) = child.stdout.take() {
        stdout
            .read_to_end(&mut answer)
            .map_err(|error| format!("cannot read the password dialog: {error}"))?;
    }
    let mut errors = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut errors);
    }
    let status = child
        .wait()
        .map_err(|error| format!("the password dialog: {error}"))?;
    match status.code() {
        Some(0) => {
            let text = String::from_utf8(answer.to_vec())
                .map_err(|_| "the password is not valid UTF-8".to_string())?;
            let text = Zeroizing::new(text);
            Ok(Some(Zeroizing::new(
                text.strip_suffix('\n').unwrap_or(&text).to_string(),
            )))
        }
        Some(CANCELLED) => Ok(None),
        _ => Err(format!(
            "the password dialog could not open: {}",
            errors.trim()
        )),
    }
}

/// `se elevated ask-password --caption C --message M`: the window.
pub(super) fn run_window(args: &[String]) -> Result<i32, String> {
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
            .unwrap_or_default()
    };
    let (caption, message) = (value("--caption"), value("--message"));
    let answer: Arc<Mutex<Option<Zeroizing<String>>>> = Arc::default();
    let window = PasswordWindow {
        message,
        password: Zeroizing::new(String::new()),
        answer: answer.clone(),
        focused: false,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{caption} – Smart Explorer"))
            .with_inner_size([440.0, 170.0])
            .with_always_on_top()
            .with_active(true),
        ..Default::default()
    };
    eframe::run_native(
        "Smart Explorer Passwort",
        options,
        Box::new(move |_| Ok(Box::new(window))),
    )
    .map_err(|error| format!("the password window cannot open: {error}"))?;
    let password = answer.lock().ok().and_then(|mut slot| slot.take());
    match password {
        Some(password) => {
            let mut stdout = std::io::stdout();
            stdout
                .write_all(password.as_bytes())
                .and_then(|()| stdout.write_all(b"\n"))
                .and_then(|()| stdout.flush())
                .map_err(|error| error.to_string())?;
            Ok(0)
        }
        None => Ok(CANCELLED),
    }
}

struct PasswordWindow {
    message: String,
    password: Zeroizing<String>,
    answer: Arc<Mutex<Option<Zeroizing<String>>>>,
    focused: bool,
}

impl eframe::App for PasswordWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut close = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(&self.message);
            ui.add_space(6.0);
            let field = ui.add(
                egui::TextEdit::singleline(&mut *self.password)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );
            if !self.focused {
                field.request_focus();
                self.focused = true;
            }
            let entered =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() || entered {
                    if let Ok(mut slot) = self.answer.lock() {
                        *slot = Some(self.password.clone());
                    }
                    close = true;
                }
                if ui.button("Abbrechen").clicked() {
                    close = true;
                }
            });
            ui.add_space(4.0);
            ui.small(
                "Smart Explorer speichert das Passwort nicht; es geht nur an sudo dieses Befehls.",
            );
        });
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
