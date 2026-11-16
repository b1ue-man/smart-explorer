//! ScreenCast v6's serial is not retained by ashpd 0.13.13's StreamProperties.
//! Keep ashpd's session/selection APIs, but decode Start's documented wire response losslessly.
use std::collections::HashMap;
use std::sync::Arc;

use ashpd::desktop::{Session, SessionPortal};
use ashpd::zbus::{
    self,
    zvariant::{as_value, OwnedObjectPath, Type, Value},
};
use futures_util::StreamExt;
use serde::Deserialize;

use super::super::pipewire_stream::identity::StreamIdentity;
use super::Refusal;

#[derive(Deserialize, Type)]
#[zvariant(signature = "dict")]
pub struct StartResults {
    #[serde(default, with = "as_value")]
    streams: Vec<(u32, StreamProperties)>,
    #[serde(default, with = "as_value::optional")]
    pub restore_token: Option<String>,
    #[serde(default, with = "as_value::optional")]
    pub devices: Option<u32>,
}

#[derive(Deserialize, Type)]
#[zvariant(signature = "dict")]
struct StreamProperties {
    #[serde(default, with = "as_value::optional")]
    id: Option<String>,
    #[serde(default, with = "as_value::optional")]
    size: Option<(i32, i32)>,
    #[serde(default, rename = "pipewire-serial", with = "as_value::optional")]
    serial: Option<u64>,
}

fn failed(error: impl std::fmt::Display) -> Refusal {
    Refusal::Failed(format!("portal_source_binding: {error}"))
}

/// How the portal shows the pointer in the stream (plan 2026-11-14 BK8a, B-16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerMode {
    /// `CursorMode::Metadata`: not in the picture, `SPA_META_Cursor` of each
    /// buffer carries it (`pipewire_cursor.rs`).
    Metadata,
    /// `CursorMode::Embedded`: in the picture, as before BK8a.
    Embedded,
    /// Neither is advertised (or the list is unreadable): the portal default
    /// (Hidden) applies, as before BK8a.
    Unspecified,
}

impl PointerMode {
    /// The session's report of the mode (`cursor_metadata`, `cursor_embedded`),
    /// written once per portal session to the host process's error output.
    pub fn report(self) -> Option<&'static str> {
        match self {
            Self::Metadata => Some("cursor_metadata"),
            Self::Embedded => Some("cursor_embedded"),
            Self::Unspecified => None,
        }
    }
}

/// The cursor mode to ask for within `AvailableCursorModes` (`metadata` and
/// `embedded`: whether the portal offers them): Metadata when offered (the
/// viewer then draws the pointer, or the host mixes it in), else Embedded,
/// else none. ScreenCast spec: `cursor_mode` "must be one of the cursor modes
/// advertised in AvailableCursorModes. Setting a cursor mode not advertised
/// will cause the screen cast session to be closed."
pub fn pick_cursor_mode(
    metadata: bool,
    embedded: bool,
) -> (Option<ashpd::desktop::screencast::CursorMode>, PointerMode) {
    use ashpd::desktop::screencast::CursorMode;
    if metadata {
        (Some(CursorMode::Metadata), PointerMode::Metadata)
    } else if embedded {
        (Some(CursorMode::Embedded), PointerMode::Embedded)
    } else {
        (None, PointerMode::Unspecified)
    }
}

/// SelectSources options within what the portal advertises. The pointer
/// mode is `pick_cursor_mode`'s; monitors and windows as far as
/// AvailableSourceTypes lists either (unreadable: both, as before). The
/// restore token is not part of these options: it names the earlier source
/// choice, and `cursor_mode` is an option of every `SelectSources` call, so
/// a stored grant is passed together with the new mode (the portal
/// documentation does not tie a restore token to a cursor mode [assumption:
/// no new dialog for the changed mode; if the portal asks anyway, the grant
/// is as invalid as any other that cannot be restored]).
pub async fn advertised_sources(
    screencast: &ashpd::desktop::screencast::Screencast,
) -> ashpd::desktop::screencast::SelectSourcesOptions {
    use ashpd::desktop::screencast::{CursorMode, SelectSourcesOptions, SourceType};
    let wanted = SourceType::Monitor | SourceType::Window;
    let types = match screencast.available_source_types().await {
        Ok(available) if !(available & wanted).is_empty() => available & wanted,
        _ => wanted,
    };
    let modes = screencast.available_cursor_modes().await.ok();
    let offers = |mode: CursorMode| modes.as_ref().is_some_and(|modes| modes.contains(mode));
    let (cursor, mode) =
        pick_cursor_mode(offers(CursorMode::Metadata), offers(CursorMode::Embedded));
    if let Some(report) = mode.report() {
        eprintln!("screen host process: portal pointer: {report}");
    }
    // Merge W2: the value for the statistics (`core::capture_route`).
    crate::screen::core::capture_route::note_cursor_embedded(mode == PointerMode::Embedded);
    SelectSourcesOptions::default()
        .set_cursor_mode(cursor)
        .set_sources(types)
        .set_multiple(false)
}

pub fn session_path<T: SessionPortal>(session: &Session<T>) -> Result<String, Refusal> {
    serde_json::to_value(session)
        .map_err(failed)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("session handle is not an object path"))
}

impl StartResults {
    pub fn selected<T: SessionPortal>(
        &self,
        session: &Session<T>,
        portal_version: u32,
    ) -> Result<Arc<StreamIdentity>, Refusal> {
        self.bind(session_path(session)?, portal_version)
    }

    fn bind(&self, session: String, portal_version: u32) -> Result<Arc<StreamIdentity>, Refusal> {
        // SelectSources(multiple=false) grants exactly one stream. Never silently take the first.
        let [(node_id, properties)] = self.streams.as_slice() else {
            return Err(failed("the portal must choose exactly one stream"));
        };
        if portal_version >= 6 && properties.serial.is_none() {
            return Err(failed(
                "ScreenCast v6 omitted pipewire-serial; choose the source again",
            ));
        }
        Ok(Arc::new(StreamIdentity::new(
            *node_id,
            properties.serial,
            session,
            properties.id.clone(),
            properties
                .size
                .filter(|(width, height)| *width > 0 && *height > 0),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize, Type)]
    #[zvariant(signature = "dict")]
    struct WireProperties {
        #[serde(
            rename = "pipewire-serial",
            with = "as_value::optional",
            skip_serializing_if = "Option::is_none"
        )]
        serial: Option<u64>,
        #[serde(with = "as_value")]
        size: (i32, i32),
    }

    #[derive(Serialize, Type)]
    #[zvariant(signature = "dict")]
    struct WireResults {
        #[serde(with = "as_value")]
        streams: Vec<(u32, WireProperties)>,
    }

    fn response(serial: Option<u64>, count: usize) -> StartResults {
        let raw = WireResults {
            streams: (0..count)
                .map(|_| {
                    (
                        42,
                        WireProperties {
                            serial,
                            size: (960, 540),
                        },
                    )
                })
                .collect(),
        };
        let message = zbus::message::Message::signal(
            "/org/freedesktop/portal/desktop/request/test",
            "org.freedesktop.portal.Request",
            "Response",
        )
        .unwrap()
        .build(&(0u32, raw))
        .unwrap();
        let (status, results): (u32, StartResults) = message.body().deserialize().unwrap();
        assert_eq!(status, 0);
        results
    }

    #[test]
    fn batches_task_p4d_start_wire_preserves_serial_and_rejects_ambiguous_source() {
        let results = response(Some(u64::MAX), 1);
        let selected = results.bind("/session/granted".into(), 6).ok().unwrap();
        assert_eq!(selected.serial, Some(u64::MAX));
        assert_eq!(selected.node_id, 42);
        assert_eq!(selected.logical_size, Some((960, 540)));
        assert_eq!(selected.session, "/session/granted");
        assert!(response(None, 1).bind("/session/legacy".into(), 5).is_ok());
        assert!(response(None, 1).bind("/session/v6".into(), 6).is_err());
        assert!(response(Some(1), 0)
            .bind("/session/none".into(), 6)
            .is_err());
        assert!(response(Some(1), 2)
            .bind("/session/multiple".into(), 6)
            .is_err());
    }

    #[test]
    fn bildkette_task_wayland_cursor_mode_metadata_only_when_advertised() {
        // Metadata wins over Embedded, but only when the portal offers it.
        let (mode, pointer) = pick_cursor_mode(true, true);
        assert!(mode.is_some());
        assert_eq!(pointer, PointerMode::Metadata);
        let (mode, pointer) = pick_cursor_mode(false, true);
        assert!(mode.is_some());
        assert_eq!(pointer, PointerMode::Embedded);
        // Nothing usable advertised, or the list unreadable: no mode is set
        // (a mode that is not advertised closes the session).
        let (mode, pointer) = pick_cursor_mode(false, false);
        assert!(mode.is_none());
        assert_eq!(pointer, PointerMode::Unspecified);
        // The report names the embedded pointer, as the plan says.
        assert_eq!(PointerMode::Embedded.report(), Some("cursor_embedded"));
        assert_eq!(PointerMode::Metadata.report(), Some("cursor_metadata"));
        assert_eq!(PointerMode::Unspecified.report(), None);
    }

    /// The stored grant goes along with the new mode: in the screen cast
    /// session the options `advertised_sources` returned get the token and
    /// the persist mode before the one `select_sources` call, so a changed
    /// pointer mode does not drop the grant.
    #[test]
    fn bildkette_task_wayland_cursor_mode_keeps_the_stored_grant_in_the_same_select_call() {
        let session = include_str!("portal_session.rs");
        let advertised = session
            .find("source_binding::advertised_sources(screencast)")
            .unwrap();
        let token = session
            .find(".set_restore_token(token.as_deref())")
            .unwrap();
        let persist = session
            .find(".set_persist_mode(PersistMode::ExplicitlyRevoked)")
            .unwrap();
        let select = session
            .find(".select_sources(session, select_options)")
            .unwrap();
        assert!(advertised < token && token < persist && persist < select);
    }
}

/// Subscribe before Start so even an immediate response cannot be missed.
/// Only the existing session is started; no second dialog/session or input transport is created.
pub async fn start<T: SessionPortal>(
    portal: &zbus::Proxy<'_>,
    session: &Session<T>,
) -> Result<StartResults, Refusal> {
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(failed)?;
    let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let token = format!("se_start_{nonce}");
    let connection = portal.connection();
    let sender = connection
        .unique_name()
        .ok_or_else(|| failed("missing D-Bus unique name"))?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let request = zbus::Proxy::new(
        connection,
        "org.freedesktop.portal.Desktop",
        path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(failed)?;
    let mut responses = request.receive_signal("Response").await.map_err(failed)?;
    let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
    let returned: OwnedObjectPath = portal
        .call("Start", &(session, "", options))
        .await
        .map_err(failed)?;
    if returned.as_str() != path {
        return Err(failed("Start returned an unexpected request handle"));
    }
    let message = responses
        .next()
        .await
        .ok_or_else(|| failed("Start response stream ended"))?;
    let (status, results): (u32, StartResults) = message.body().deserialize().map_err(failed)?;
    match status {
        0 => Ok(results),
        1 => Err(Refusal::Denied("Cancelled".into())),
        2 => Err(Refusal::Denied("Other".into())),
        _ => Err(failed("unknown Start response code")),
    }
}
