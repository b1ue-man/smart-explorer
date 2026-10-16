//! Changing a saved remote from the desktop: "Verbindung bearbeiten…" opens
//! the connection dialog with the saved values, "Speichern" stores them
//! through `connect::save_edited_connection` (the path of the Android form and
//! `se connections edit`), and "SSH-Hostschlüssel vergessen" removes the
//! pinned key like `se connections forget-host-key`.
use super::*;
use crate::connect::ConnectForm;
use crate::creds::{Protocol, SavedConnection};

/// The port of the form as the connector reads it: empty = the protocol's
/// standard port; a UNC share has none.
fn form_port(form: &ConnectForm) -> Result<u16, String> {
    if form.port.trim().is_empty() {
        return Ok(form.protocol.default_port());
    }
    match form.port.trim().parse::<u16>() {
        Ok(port) if port > 0 || form.protocol == Protocol::Share => Ok(port),
        _ => Err("Ungültiger Port (erwartet: 1–65535)".into()),
    }
}

/// The checks the connector would make before it can save: a host for URL
/// protocols, the share at the start of an SMB root, a key file for key login.
fn check_form(form: &ConnectForm) -> Result<u16, String> {
    let port = form_port(form)?;
    if form.protocol == Protocol::Share {
        if crate::net::share_root(form.unc.trim()).is_none() {
            return Err("Die Freigabe braucht die Form \\\\server\\freigabe.".into());
        }
    } else if form.host.trim().is_empty() {
        return Err("Host fehlt.".into());
    }
    if form.protocol == Protocol::Smb && !crate::smb::root_has_share(&form.root) {
        return Err("Bei SMB beginnt der Startpfad mit der Freigabe („/freigabe/ordner“).".into());
    }
    if form.protocol == Protocol::Sftp && form.use_key && form.keyfile.trim().is_empty() {
        return Err("Bitte eine Schlüsseldatei wählen.".into());
    }
    Ok(port)
}

impl App {
    /// Sidebar and menu entry "Verbindung bearbeiten…".
    pub(in crate::app) fn open_connection_edit(&mut self, connection: &SavedConnection) {
        self.connect_form = ConnectForm::from_saved(connection);
        self.connect_edit = Some(connection.clone());
        self.connect_rx = None;
        self.connecting = false;
        self.show_connect = true;
    }

    /// "Speichern" of the dialog in edit mode. An empty password or
    /// passphrase keeps the stored one; open connections keep their settings
    /// until they reconnect.
    pub(in crate::app) fn save_connection_edit(&mut self) {
        let Some(previous) = self.connect_edit.clone() else {
            return;
        };
        let form = self.connect_form.clone();
        let port = match check_form(&form) {
            Ok(port) => port,
            Err(error) => {
                self.error_msg = Some(format!("Verbindung speichern: {error}"));
                return;
            }
        };
        let saved = crate::connect::build_saved(&form, port);
        let secret = if form.use_key {
            form.passphrase.clone()
        } else {
            form.password.clone()
        };
        match crate::connect::save_edited_connection(Some(&previous), saved, Some(&secret)) {
            Ok(edit) => {
                self.saved_connections = crate::creds::load_connections();
                self.notice = Some((
                    format!(
                        "✓ Verbindung „{}“ gespeichert – gilt ab der nächsten Verbindung",
                        edit.saved.display()
                    ),
                    std::time::Instant::now(),
                ));
                if !edit.warnings.is_empty() {
                    self.error_msg = Some(edit.warnings.join("; "));
                }
                self.show_connect = false;
                self.connect_edit = None;
                self.connect_form = ConnectForm::default();
            }
            Err(error) => self.error_msg = Some(format!("Verbindung speichern: {error}")),
        }
    }

    /// "SSH-Hostschlüssel vergessen": the next connection asks again.
    pub(in crate::app) fn forget_saved_host_key(&mut self, connection: &SavedConnection) {
        let target = format!("{}:{}", connection.host, connection.port);
        match crate::sftp::forget_host_key(&connection.host, connection.port) {
            Ok(true) => {
                self.notice = Some((
                    format!("SSH-Hostschlüssel von {target} vergessen; die nächste Verbindung fragt erneut"),
                    std::time::Instant::now(),
                ));
            }
            Ok(false) => {
                self.notice = Some((
                    format!("Für {target} war kein SSH-Hostschlüssel gespeichert"),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.error_msg = Some(format!("SSH-Hostschlüssel von {target}: {error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luecken_task_connection_dialog_checks_like_the_connector() {
        let mut form = ConnectForm {
            protocol: Protocol::Smb,
            host: "nas".into(),
            port: String::new(),
            root: "/".into(),
            ..ConnectForm::default()
        };
        assert!(check_form(&form).is_err());
        form.root = "/public/docs".into();
        assert_eq!(check_form(&form), Ok(445));
        form.port = "0".into();
        assert!(check_form(&form).is_err());
        form.protocol = Protocol::Sftp;
        form.port = "2222".into();
        form.use_key = true;
        assert!(check_form(&form).is_err());
        form.keyfile = "/k/id".into();
        assert_eq!(check_form(&form), Ok(2222));
        form.host.clear();
        assert!(check_form(&form).is_err());
    }
}
