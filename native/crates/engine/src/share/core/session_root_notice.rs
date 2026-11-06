//! R2 (plan 2026-11-06): an authorized session whose root lists nothing
//! learns why instead of seeing an empty folder. The notice names only the
//! relation's own state (no folder of another relation, no path); the
//! listing itself and every other check stay as they are.
use std::io;

use super::super::types::ShareAuthState;
use super::IncomingSession;

/// Why the root of an authorized session is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmptyRoot {
    /// The host exports folders, but none for this Direct device (rights
    /// still open after a pairing, or „Keine Ordner“ chosen).
    NoFoldersForDevice,
    /// The host exports nothing at all over Direct.
    NothingShared,
    /// The room carries no folder of this host.
    RoomWithoutFolders,
}

impl EmptyRoot {
    fn message(self, host: &str) -> String {
        let host = if host.trim().is_empty() {
            "Das andere Gerät"
        } else {
            host.trim()
        };
        match self {
            Self::NoFoldersForDevice => format!(
                "{host} hat diesem Gerät noch keine Ordner freigegeben (Rechte offen oder „Keine Ordner“ gewählt). Auf {host} unter „Geräte & Freigaben“ dieses Gerät öffnen und unter „Rechte festlegen“ Ordner wählen."
            ),
            Self::NothingShared => format!(
                "{host} gibt keine Ordner frei. Auf {host} unter „Geräte & Freigaben“ einen Ordner freigeben."
            ),
            Self::RoomWithoutFolders => format!(
                "{host} gibt in diesem Raum keine Ordner frei. Auf {host} unter „Geräte & Freigaben“ den Raum öffnen und unter „Rechte festlegen“ Ordner wählen."
            ),
        }
    }
}

impl IncomingSession {
    /// The reason an authorized root listing came back empty, judged on the
    /// current state; `None` when the session is no longer authorized (the
    /// caller's own check reports that) or nothing explains it.
    fn empty_root(&self, state: &ShareAuthState) -> Option<EmptyRoot> {
        let hello = &self.hello;
        match hello.relation_kind.as_str() {
            "direct" => {
                let grant = state.direct_grants.iter().find(|grant| {
                    grant.authorizes_session(&hello.device_id, &hello.public_key, &hello.node_id)
                })?;
                let exports = &state.default_direct_exports;
                let visible = exports.visible_to(&grant.device_id, &grant.public_key);
                if !visible.roots.is_empty() || visible.shares_connections() {
                    return None;
                }
                Some(
                    if exports.roots.is_empty() && !exports.shares_connections() {
                        EmptyRoot::NothingShared
                    } else {
                        EmptyRoot::NoFoldersForDevice
                    },
                )
            }
            "room" => {
                let room = state
                    .rooms
                    .iter()
                    .find(|room| room.room_id == hello.relation_id && room.auto_join)?;
                (room.exports.roots.is_empty() && !room.exports.shares_connections())
                    .then_some(EmptyRoot::RoomWithoutFolders)
            }
            _ => None,
        }
    }

    /// `empty_root` as the error the root listing answers with: access
    /// denied, with the reason and the way to the folders.
    pub(in crate::share) fn empty_root_error(&self, state: &ShareAuthState) -> Option<io::Error> {
        let reason = self.empty_root(state)?;
        Some(io::Error::new(
            io::ErrorKind::PermissionDenied,
            reason.message(&state.identity.device_name),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::EmptyRoot;

    #[test]
    fn rechte_task_empty_root_messages_name_the_way() {
        let text = EmptyRoot::NoFoldersForDevice.message("PC");
        assert!(
            text.starts_with("PC hat diesem Gerät noch keine Ordner freigegeben"),
            "{text}"
        );
        assert!(text.contains("Rechte festlegen"), "{text}");
        assert!(EmptyRoot::NothingShared
            .message(" ")
            .starts_with("Das andere Gerät"));
        assert!(EmptyRoot::RoomWithoutFolders.message("PC").contains("Raum"));
    }
}
