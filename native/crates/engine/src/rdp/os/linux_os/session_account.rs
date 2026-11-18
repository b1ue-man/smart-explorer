//! The account at this device's desktop for a root worker (RDP-ZIELKONTO,
//! open point 7): logind's sessions, weighed by
//! `core::xrdp_login::desktop_user`. A root worker's „own“ account would be
//! `root`, which xrdp refuses unless `AllowRootLogin` is set.
use crate::rdp::core::xrdp_login::{desktop_user, DesktopSession};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedObjectPath,
};

const LOGIND: &str = "org.freedesktop.login1";

/// logind `ListSessions`: (id, uid, user, seat, object path).
type Listed = (String, u32, String, String, OwnedObjectPath);

/// The user of the active local graphical session, if logind names one.
pub(super) fn desktop_account() -> Option<String> {
    let connection = Connection::system().ok()?;
    let manager = Proxy::new(
        &connection,
        LOGIND,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .ok()?;
    let listed: Vec<Listed> = manager.call("ListSessions", &()).ok()?;
    let sessions: Vec<DesktopSession> = listed
        .into_iter()
        .filter(|(_, uid, ..)| *uid != 0)
        .filter_map(|(_, uid, user, seat, path)| {
            let session = Proxy::new(
                &connection,
                LOGIND,
                path.as_str(),
                "org.freedesktop.login1.Session",
            )
            .ok()?;
            Some(DesktopSession {
                uid,
                user,
                seat,
                active: session.get_property::<bool>("Active").ok()?,
                remote: session.get_property::<bool>("Remote").ok()?,
                class: session.get_property::<String>("Class").ok()?,
                kind: session.get_property::<String>("Type").ok()?,
            })
        })
        .collect();
    desktop_user(&sessions)
}
