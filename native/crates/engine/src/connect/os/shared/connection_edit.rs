//! Saving a changed saved connection: one path for the Android connection
//! form (`conn.save` with an id), the desktop dialog "Verbindung bearbeiten"
//! and `se connections edit`. Host, port, user and root form the account key
//! (`SavedConnection::account`); when they change, the entry moves and takes
//! its stored secret along unless the kind of secret changed — a password is
//! never used as a key passphrase or the other way round.
use crate::creds::{AuthKind, SavedConnection};

/// What `save_edited_connection` did; `warnings` lists cleanup steps that
/// failed after the changed entry was saved and, after a move, the sync setups
/// and drives still on the old address (shown, never fatal).
pub struct ConnectionEdit {
    pub saved: SavedConnection,
    /// The previous account key when the entry moved.
    pub moved_from: Option<String>,
    pub warnings: Vec<String>,
}

/// The decisions of an edit, without touching the stores.
#[derive(Debug, PartialEq, Eq)]
struct EditPlan {
    moved_from: Option<String>,
    /// Read the secret of the previous account and store it under the new one.
    carry_secret: bool,
    /// Delete a secret of the new account that no longer fits its login kind.
    drop_unfitting_secret: bool,
}

/// Password and key passphrase are different secrets; a key file that only
/// moved keeps its passphrase. An SSH-agent login has no secret.
pub fn same_secret_kind(left: &AuthKind, right: &AuthKind) -> bool {
    matches!(
        (left, right),
        (AuthKind::Password, AuthKind::Password) | (AuthKind::Key { .. }, AuthKind::Key { .. })
    )
}

fn plan(
    previous: Option<&SavedConnection>,
    saved: &SavedConnection,
    secret_given: bool,
) -> EditPlan {
    let account = saved.account();
    let moved_from = previous
        .map(SavedConnection::account)
        .filter(|old| *old != account);
    let kind_changed =
        previous.is_some_and(|previous| !same_secret_kind(&previous.auth, &saved.auth));
    EditPlan {
        carry_secret: !secret_given && moved_from.is_some() && !kind_changed,
        drop_unfitting_secret: !secret_given && kind_changed,
        moved_from,
    }
}

/// Saves `saved` in place of `previous` (`None`: a new connection). An empty
/// or missing `secret` keeps the stored one where its kind still fits; the
/// previous entry is removed after the new one is saved.
pub fn save_edited_connection(
    previous: Option<&SavedConnection>,
    saved: SavedConnection,
    secret: Option<&str>,
) -> Result<ConnectionEdit, String> {
    let given = secret.filter(|secret| !secret.is_empty());
    let plan = plan(previous, &saved, given.is_some());
    let account = saved.account();
    let carried = match (&plan.moved_from, plan.carry_secret) {
        (Some(old), true) => crate::creds::get_secret_checked(old)?,
        _ => None,
    };
    crate::creds::save_connection_with_secret(&saved, given.or(carried.as_deref()))?;
    let mut warnings = Vec::new();
    if plan.drop_unfitting_secret {
        if let Err(error) = crate::creds::delete_secret_checked(&account) {
            warnings.push(format!("Altes Geheimnis nicht gelöscht: {error}"));
        }
    }
    if let Some(old) = &plan.moved_from {
        if let Err(error) = crate::creds::remove_connection(old) {
            warnings.push(format!("Alte Verbindung nicht entfernt: {error}"));
        }
        if let Some(previous) = previous {
            warnings.extend(super::connection_edit_refs::references_left_behind(
                previous, &saved,
            ));
        }
    }
    Ok(ConnectionEdit {
        saved,
        moved_from: plan.moved_from,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creds::Protocol;

    fn connection(host: &str, auth: AuthKind) -> SavedConnection {
        SavedConnection {
            protocol: Protocol::Sftp,
            host: host.into(),
            port: 22,
            user: "alice".into(),
            auth,
            root: "/srv".into(),
            label: "nas".into(),
            use_agent: false,
        }
    }

    #[test]
    fn luecken_task_connection_edit_moves_and_keeps_its_secret() {
        let old = connection("nas", AuthKind::Password);
        let new = connection("nas.lan", AuthKind::Password);
        assert_eq!(
            plan(Some(&old), &new, false),
            EditPlan {
                moved_from: Some("sftp://alice@nas:22/srv".into()),
                carry_secret: true,
                drop_unfitting_secret: false,
            }
        );
        // A new password is stored instead of the old one.
        assert!(!plan(Some(&old), &new, true).carry_secret);
    }

    #[test]
    fn luecken_task_connection_edit_never_reuses_a_password_as_passphrase() {
        let old = connection("nas", AuthKind::Password);
        let key = connection(
            "nas",
            AuthKind::Key {
                path: "/k/id".into(),
            },
        );
        let edit = plan(Some(&old), &key, false);
        assert_eq!(edit.moved_from, None);
        assert!(!edit.carry_secret);
        assert!(edit.drop_unfitting_secret);
        // A moved key file keeps its passphrase.
        let moved = connection(
            "nas",
            AuthKind::Key {
                path: "/other/id".into(),
            },
        );
        let edit = plan(Some(&key), &moved, false);
        assert!(!edit.drop_unfitting_secret);
        assert_eq!(edit.moved_from, None);
        // SSH agent logins have no secret to carry.
        let agent = connection("nas.lan", AuthKind::SshAgent);
        let edit = plan(Some(&old), &agent, false);
        assert!(!edit.carry_secret && edit.drop_unfitting_secret);
    }

    #[test]
    fn luecken_task_connection_edit_of_a_new_entry_changes_nothing_else() {
        let new = connection("nas", AuthKind::Password);
        assert_eq!(
            plan(None, &new, false),
            EditPlan {
                moved_from: None,
                carry_secret: false,
                drop_unfitting_secret: false,
            }
        );
    }
}
