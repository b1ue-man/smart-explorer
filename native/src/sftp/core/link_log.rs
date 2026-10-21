//! One line in the service log (`daemon.log`) for every extra SSH connection
//! that logs in, that closes and for every block (security answer E13-S15,
//! question 3). Time, host, port, user, login method, open connections and
//! the reason; never a password, passphrase or key material.

/// Who an extra connection belongs to.
#[derive(Clone, Debug)]
pub(in crate::sftp) struct LinkIdent {
    pub(in crate::sftp) host: String,
    pub(in crate::sftp) port: u16,
    pub(in crate::sftp) user: String,
    /// `password`, `keyboard-interactive` or `publickey`; `-` before a login.
    pub(in crate::sftp) method: String,
}

/// The text of one log line (the log adds the time).
pub(in crate::sftp) fn line(event: &str, ident: &LinkIdent, open: usize, reason: &str) -> String {
    format!(
        "SFTP-Zusatzverbindung {event}: Host {}, Port {}, Benutzer {}, Methode {}, \
         offene Verbindungen {open}, Grund: {}",
        printable(&ident.host),
        ident.port,
        printable(&ident.user),
        printable(&ident.method),
        printable(reason),
    )
}

/// No control characters: a server's error text cannot split a log line.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Writes the line to the service log (`crate::daemon::log_worker`, which locks
/// across processes). Tests do not touch the user's log.
pub(in crate::sftp) fn emit(line: &str) {
    #[cfg(not(test))]
    crate::daemon::log_worker(line);
    #[cfg(test)]
    let _ = line;
}
