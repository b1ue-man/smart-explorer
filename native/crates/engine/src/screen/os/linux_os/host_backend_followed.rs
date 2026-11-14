//! The daemon side of the Linux screen host for a worker that follows the
//! active session (E23, E47, S3). Such a worker is a root service or a
//! worker without session variables: it never connects to the session's
//! compositor or session bus itself. Window list, icons and their theme come
//! from a short host process in the session's account
//! (`host_worker_client::session_queries`), which reads the account's
//! desktop files and asks its compositor and its systemd instance as that
//! account; the portal tokens are read here from the account's home, only
//! as the account's own file without a link (`account`). A worker with its
//! own session variables keeps every path of `host_backend` unchanged.
use std::io;

use super::{unknown_list, LIMIT_NO_WINDOW_LIST, LIMIT_WINDOW_PICK_ON_HOST};
use crate::screen::core::app_icon::{AuthorizedIconEntry, IconReply};
use crate::screen::core::window_list::{ListSource, WindowList};
use crate::screen::os::host_worker_client::session_queries::{self, SessionAnswer, SessionQuery};
use crate::screen::os::portal_tokens::Tokens;
use crate::screen::os::session_detect::SessionKind;

#[path = "followed_account.rs"]
pub(in crate::screen::os) mod account;

/// The window list of the followed session of kind `kind`.
pub(super) fn windows(kind: SessionKind) -> WindowList {
    windows_from(kind, || session_queries::ask(&SessionQuery::Windows))
}

/// The window list of the followed session as `ask` answers it: the
/// account's list with its limits; without an answer (or without a session)
/// an unknown list with the limits a list of that kind would have.
pub(in crate::screen::os) fn windows_from(
    kind: SessionKind,
    ask: impl FnOnce() -> io::Result<SessionAnswer>,
) -> WindowList {
    let mut list = match kind {
        SessionKind::None => unknown_list(),
        SessionKind::X11 | SessionKind::Wayland => {
            session_queries::windows_of(ask()).unwrap_or_else(|_| unknown_list())
        }
    };
    let wanted = [
        (kind == SessionKind::Wayland, LIMIT_WINDOW_PICK_ON_HOST),
        (
            list.list_source == ListSource::Unknown,
            LIMIT_NO_WINDOW_LIST,
        ),
    ];
    for (applies, key) in wanted {
        if applies && !list.limits.iter().any(|known| known == key) {
            list.limits.push(key.into());
        }
    }
    list
}

/// Icons of a list made here: in this process, unless the backend follows
/// a session (`follows`); then the account's process attached them.
pub(super) fn attach(follows: bool, list: &mut WindowList) {
    if !follows {
        super::super::app_icons::attach(list);
    }
}

/// The icon of an authorized entry: in this process, or for a backend that
/// follows a session from the account's process, only when it is the icon
/// the authorized list names.
pub(super) fn icon(follows: bool, entry: AuthorizedIconEntry<'_>) -> io::Result<Option<IconReply>> {
    if !follows {
        return super::super::app_icons::load(entry);
    }
    let (query, expected) = session_queries::icon_query(entry);
    let Some(expected) = expected else {
        return Ok(None);
    };
    session_queries::icon_of(session_queries::ask(&query), &expected)
}

/// The portal tokens of the followed session's account.
pub(super) fn stored_tokens() -> Tokens {
    account::followed_tokens()
}
