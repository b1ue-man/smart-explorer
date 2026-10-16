//! "Einstellungen → Integration → KI-Agenten": the AI agent plugin of
//! `se plugin` (Claude Code plugin, Codex MCP entry and skill) with status,
//! install and remove per host, and the configuration for other MCP clients.
//! The panel runs the installed `se` itself, so the GUI and the terminal set
//! up exactly the same files and host entries (`se` beside the app).
use super::prelude::*;
use super::*;
use serde_json::Value;

/// MCP clients `se plugin config` knows (`cli/plugin/config.rs`).
const CONFIG_HOSTS: [(&str, &str); 5] = [
    ("generic", "Cursor, Gemini CLI, Claude Desktop (mcpServers)"),
    ("vscode", "VS Code (mcp.json)"),
    ("opencode", "OpenCode (opencode.json)"),
    ("claude", "Claude Code ohne Skill (claude mcp add)"),
    ("codex", "Codex (config.toml)"),
];

/// What a finished `se plugin …` run printed.
struct SeRun {
    action: &'static str,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Default)]
pub(super) struct AiPluginPanel {
    rx: Option<Receiver<Result<SeRun, String>>>,
    running: Option<&'static str>,
    status: Option<Result<Vec<Value>, String>>,
    message: Option<String>,
    config_host: usize,
    config_text: Option<String>,
}

/// One status line per host from `se plugin status --json`.
fn host_line(report: &Value) -> String {
    let found = |cli: &Value| {
        if cli.is_null() {
            "nicht gefunden".to_string()
        } else {
            format!("gefunden ({})", cli.as_str().unwrap_or("?"))
        }
    };
    let files = |state: &Value| match (state["generated"].as_bool(), state["current"].as_bool()) {
        (Some(true), Some(true)) => "erzeugt, aktuell",
        (Some(true), _) => "erzeugt, veraltet (wird beim nächsten Start von se mcp erneuert)",
        _ => "nicht erzeugt",
    };
    match report["host"].as_str() {
        Some("claude") => format!(
            "Claude Code: {} · Plugin {} · Dateien {}",
            found(&report["cli"]),
            match report["plugin_listed"].as_bool() {
                Some(true) => "installiert",
                Some(false) => "nicht installiert",
                None => "unbekannt",
            },
            files(&report["files"])
        ),
        Some("codex") => format!(
            "Codex: {} · MCP-Eintrag {} · Skill {}",
            found(&report["cli"]),
            match (
                report["server"]["present"].as_bool(),
                report["server"]["current"].as_bool()
            ) {
                (Some(true), Some(true)) => "vorhanden, aktuell",
                (Some(true), _) => "vorhanden, anderer se-Pfad",
                (Some(false), _) => "fehlt",
                _ => "unbekannt",
            },
            files(&report["skill"])
        ),
        _ => report.to_string(),
    }
}

/// The outcome line of `se plugin install|uninstall --json` per host.
fn outcome_line(report: &Value) -> String {
    let host = report["host"].as_str().unwrap_or("?");
    if let Some(error) = report["error"].as_str() {
        return format!("{host}: {error}");
    }
    match report["installed"].as_bool() {
        Some(true) => format!("{host}: installiert – neue Agenten-Sitzung starten"),
        Some(false) => format!("{host}: nicht installiert"),
        None => format!("{host}: entfernt"),
    }
}

fn run_se(action: &'static str, arguments: Vec<String>) -> Result<SeRun, String> {
    let se = crate::updater::installed_cli_path()?;
    if !se.is_file() {
        return Err(format!("se nicht gefunden: {}", se.display()));
    }
    let mut command = std::process::Command::new(&se);
    command
        .args(&arguments)
        .env_remove("COMPLETE")
        .stdin(std::process::Stdio::null());
    hide_child_console(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("{} starten: {error}", se.display()))?;
    Ok(SeRun {
        action,
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

impl AiPluginPanel {
    fn start(&mut self, action: &'static str, arguments: &[&str]) {
        let arguments: Vec<String> = arguments.iter().map(|value| value.to_string()).collect();
        let (tx, rx) = unbounded();
        let spawned = std::thread::Builder::new()
            .name("se-plugin".into())
            .spawn(move || {
                let _ = tx.send(run_se(action, arguments));
            });
        match spawned {
            Ok(_) => {
                self.rx = Some(rx);
                self.running = Some(action);
            }
            Err(error) => self.message = Some(format!("se konnte nicht starten: {error}")),
        }
    }

    /// Takes a finished run; returns `true` when a status refresh should follow.
    fn drain(&mut self) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return false,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Err("se plugin endete ohne Ergebnis".to_string())
            }
        };
        self.rx = None;
        self.running = None;
        let run = match result {
            Ok(run) => run,
            Err(error) => {
                self.message = Some(error);
                return false;
            }
        };
        let reports = serde_json::from_str::<Vec<Value>>(&run.stdout);
        match run.action {
            "status" => {
                self.status = Some(reports.map_err(|_| failure(&run)));
                false
            }
            "config" => {
                self.config_text = Some(if run.code == Some(0) {
                    run.stdout
                } else {
                    failure(&run)
                });
                false
            }
            _ => {
                self.message = Some(match reports {
                    Ok(reports) => reports
                        .iter()
                        .map(outcome_line)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    Err(_) => failure(&run),
                });
                true
            }
        }
    }
}

fn failure(run: &SeRun) -> String {
    let text = if run.stderr.trim().is_empty() {
        run.stdout.trim()
    } else {
        run.stderr.trim()
    };
    format!("se plugin {} (Exit {:?}): {text}", run.action, run.code)
}

impl App {
    pub(super) fn ui_settings_ai_plugin(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("KI-Agenten (Claude Code, Codex)")
            .id_salt("settings_ai_plugin")
            .show(ui, |ui| {
                let panel = &mut self.settings.ai_plugin;
                if panel.drain() {
                    panel.start("status", &["plugin", "status", "--json"]);
                }
                if panel.status.is_none() && panel.rx.is_none() {
                    panel.start("status", &["plugin", "status", "--json"]);
                }
                ui.label(
                    RichText::new(
                        "Agenten starten `se mcp` selbst und nutzen Smart Explorer für Dateien, \
                         gespeicherte Verbindungen, Share-Geräte und Befehle. Gleich wie `se plugin` im Terminal.",
                    )
                    .small()
                    .color(theme::muted(ui)),
                );
                match &panel.status {
                    Some(Ok(reports)) => {
                        for report in reports {
                            ui.label(host_line(report));
                        }
                    }
                    Some(Err(error)) => {
                        ui.colored_label(theme::danger(ui), error);
                    }
                    None => {}
                }
                let idle = panel.running.is_none();
                ui.horizontal_wrapped(|ui| {
                    for (label, arguments) in [
                        ("In alle gefundenen installieren", ["plugin", "install", "all", "--json"]),
                        ("Nur Claude Code", ["plugin", "install", "claude", "--json"]),
                        ("Nur Codex", ["plugin", "install", "codex", "--json"]),
                    ] {
                        if ui.add_enabled(idle, egui::Button::new(label)).clicked() {
                            panel.start("install", &arguments);
                        }
                    }
                    if ui.add_enabled(idle, egui::Button::new("Entfernen")).on_hover_text("Plugin, MCP-Eintrag und erzeugte Dateien aus Claude Code und Codex entfernen").clicked() {
                        panel.start("uninstall", &["plugin", "uninstall", "all", "--json"]);
                    }
                    if ui.add_enabled(idle, egui::Button::new("Status prüfen")).clicked() {
                        panel.start("status", &["plugin", "status", "--json"]);
                    }
                });
                if let Some(action) = panel.running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("se plugin {action} läuft…"));
                    });
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                }
                if let Some(message) = &panel.message {
                    ui.label(message);
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Andere MCP-Clients:");
                    egui::ComboBox::from_id_salt("ai_plugin_config_host")
                        .selected_text(CONFIG_HOSTS[panel.config_host].1)
                        .show_ui(ui, |ui| {
                            for (index, (_, label)) in CONFIG_HOSTS.iter().enumerate() {
                                ui.selectable_value(&mut panel.config_host, index, *label);
                            }
                        });
                    if ui.add_enabled(idle, egui::Button::new("Konfiguration zeigen")).clicked() {
                        let host = CONFIG_HOSTS[panel.config_host].0;
                        panel.start("config", &["plugin", "config", host]);
                    }
                });
                if let Some(text) = &mut panel.config_text {
                    ui.add(egui::TextEdit::multiline(text).code_editor().desired_rows(4).desired_width(f32::INFINITY));
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::{host_line, outcome_line};
    use serde_json::json;

    #[test]
    fn luecken_task_ai_plugin_panel_parses_se_reports() {
        let claude = json!({"host": "claude", "cli": "/usr/bin/claude", "plugin_listed": true,
            "files": {"generated": true, "current": true}});
        assert_eq!(
            host_line(&claude),
            "Claude Code: gefunden (/usr/bin/claude) · Plugin installiert · Dateien erzeugt, aktuell"
        );
        let codex = json!({"host": "codex", "cli": null, "server": null,
            "skill": {"generated": false}});
        assert_eq!(
            host_line(&codex),
            "Codex: nicht gefunden · MCP-Eintrag unbekannt · Skill nicht erzeugt"
        );
        assert_eq!(
            outcome_line(&json!({"host": "codex", "installed": true})),
            "codex: installiert – neue Agenten-Sitzung starten"
        );
        assert_eq!(
            outcome_line(&json!({"host": "claude", "installed": false, "error": "not found"})),
            "claude: not found"
        );
        assert_eq!(
            outcome_line(&json!({"host": "claude", "removed_files": true, "steps": []})),
            "claude: entfernt"
        );
    }
}
