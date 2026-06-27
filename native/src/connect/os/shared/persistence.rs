use crate::connect::endpoint::norm_root;
use crate::connect::ConnectForm;
use crate::creds::{AuthKind, Protocol, SavedConnection};

/// Build the `SavedConnection` metadata (no secret) for persistence.
pub fn build_saved(form: &ConnectForm, port: u16) -> SavedConnection {
    let auth = if form.use_key {
        AuthKind::Key {
            path: form.keyfile.trim().to_string(),
        }
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

pub(super) fn persist(form: &ConnectForm, port: u16, secret: Option<&str>) {
    if !form.save {
        return;
    }
    let saved = build_saved(form, port);
    let _ = crate::creds::save_connection(&saved);
    if let Some(s) = secret {
        if !s.is_empty() {
            let _ = crate::creds::set_secret(&saved.account(), s);
        }
    }
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
    }
}
