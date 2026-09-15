//! Plain-text diagnostics keep scan context and each path's cause together.
use super::App;
use crate::vfs::Scheme;

impl App {
    pub(in crate::app) fn error_log_text(&self) -> String {
        let mut lines = Vec::new();
        if !self.app_errors.is_empty() {
            lines.push("App-Fehler:".to_string());
            for entry in &self.app_errors {
                lines.push(format!("[{}] {}: {}", entry.ts, entry.context, detail(&entry.detail)));
            }
        }
        if let Some(current) = &self.error_msg {
            if !self.app_errors.iter().any(|entry| entry.detail == *current) {
                if lines.is_empty() { lines.push("App-Fehler:".to_string()); }
                lines.push(format!("[aktuell] Fehler: {}", detail(current)));
            }
        }
        if !self.failed_paths.is_empty() || self.progress.errors > 0 {
            if !lines.is_empty() { lines.push(String::new()); }
            let total = self.progress.errors.max(self.failed_paths.len() as u64);
            lines.push(format!("Scan-Fehler: {total} gesamt, {} Pfade im Protokoll", self.failed_paths.len()));
            for (path, message) in &self.failed_paths {
                lines.push(String::new());
                lines.push(format!("Pfad: {path}"));
                lines.push(format!("Ursache: {}", detail(message)));
            }
            let omitted = total.saturating_sub(self.failed_paths.len() as u64);
            if omitted > 0 {
                lines.push(format!("Weitere Fehler ohne gespeicherten Pfad: {omitted}"));
            }
        }
        if lines.is_empty() { return String::new(); }
        let scheme = self.remote.as_ref().map(|remote| remote.backend.scheme()).unwrap_or(Scheme::Local);
        let source = match scheme {
            Scheme::Local => "Dateisystem",
            Scheme::Sftp => "SFTP / SSH",
            Scheme::Ftp => "FTP / FTPS",
            Scheme::Webdav => "WebDAV",
            Scheme::GDrive => "Google Drive",
            Scheme::Peer => "Gerätefreigabe",
        };
        let root = if self.root_path.is_empty() { "Keine aktive Scan-Wurzel" } else { &self.root_path };
        format!("Smart Explorer {}\r\nScan-Quelle: {source}\r\nScan-Wurzel: {root}\r\n\r\n{}",
            env!("CARGO_PKG_VERSION"), lines.join("\r\n"))
    }
}

fn detail(message: &str) -> &str {
    if message.trim().is_empty() {
        "Der Vorgang hat keinen Fehlertext übermittelt."
    } else {
        message
    }
}
