use crate::connect::endpoint::norm_root;
use crate::connect::ConnectForm;
use crate::creds::{AuthKind, Protocol, SavedConnection};

/// Build the `SavedConnection` metadata (no secret) for persistence.
pub fn build_saved(form: &ConnectForm, port: u16) -> SavedConnection {
    let auth = if form.use_key {
        AuthKind::Key {
            path: form.keyfile.trim().to_string(),
        }
    } else if form.use_ssh_agent && form.protocol == Protocol::Sftp {
        AuthKind::SshAgent
    } else {
        AuthKind::Password
    };
    let root = match form.protocol {
        Protocol::Share => form.unc.trim().to_string(),
        _ => norm_root(&form.root),
    };
    SavedConnection {
        protocol: form.protocol,
        host: form.host.trim().to_string(),
        port,
        user: form.user.trim().to_string(),
        auth,
        root,
        label: form.label.trim().to_string(),
        use_agent: form.use_agent && form.protocol == Protocol::Sftp,
    }
}

pub(super) fn persist(form: &ConnectForm, port: u16, secret: Option<&str>) -> Result<(), String> {
    if !form.save {
        return Ok(());
    }
    let saved = build_saved(form, port);
    crate::creds::save_connection_with_secret(&saved, secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_saved_password_and_key() {
        let mut f = ConnectForm {
            host: "h".into(),
            user: "u".into(),
            root: "data".into(),
            ..Default::default()
        };
        let s = build_saved(&f, 22);
        assert_eq!(s.protocol, Protocol::Sftp);
        assert_eq!(s.root, "/data");
        assert_eq!(s.auth, AuthKind::Password);

        f.use_key = true;
        f.keyfile = "C:/k".into();
        let s2 = build_saved(&f, 22);
        assert_eq!(
            s2.auth,
            AuthKind::Key {
                path: "C:/k".into()
            }
        );

        f.use_key = false;
        f.use_ssh_agent = true;
        assert_eq!(build_saved(&f, 22).auth, AuthKind::SshAgent);
        assert!(ConnectForm::from_saved(&build_saved(&f, 22)).use_ssh_agent);
        f.protocol = Protocol::Ftp;
        assert_eq!(build_saved(&f, 21).auth, AuthKind::Password);
    }

    #[test]
    fn luecken_task_saved_connections_of_0_5_180_open_with_the_same_login() {
        let mut saved = SavedConnection {
            protocol: Protocol::Sftp,
            host: "nas".into(),
            port: 22,
            user: "alice".into(),
            auth: AuthKind::Password,
            root: "/home/alice".into(),
            label: String::new(),
            use_agent: false,
        };
        let form = ConnectForm::from_saved(&saved);
        assert!(!form.use_key && !form.use_ssh_agent);
        assert_eq!(build_saved(&form, 22).account(), saved.account());
        assert_eq!(build_saved(&form, 22).auth, AuthKind::Password);

        saved.auth = AuthKind::Key {
            path: "/home/alice/.ssh/id".into(),
        };
        let form = ConnectForm::from_saved(&saved);
        assert!(form.use_key && !form.use_ssh_agent);
        assert_eq!(build_saved(&form, 22).auth, saved.auth);
        assert_eq!(build_saved(&form, 22).account(), saved.account());
    }

    #[test]
    fn persistence_is_a_noop_when_save_is_disabled() {
        let form = ConnectForm {
            save: false,
            ..Default::default()
        };
        assert!(persist(&form, 22, Some("not-written")).is_ok());
    }
}
