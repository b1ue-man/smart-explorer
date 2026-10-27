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
pub(crate) struct StartResults {
    #[serde(default, with = "as_value")]
    streams: Vec<(u32, StreamProperties)>,
    #[serde(default, with = "as_value::optional")]
    pub(crate) restore_token: Option<String>,
    #[serde(default, with = "as_value::optional")]
    pub(crate) devices: Option<u32>,
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

/// SelectSources options within what the portal advertises. The ScreenCast
/// spec: `cursor_mode` "must be one of the cursor modes advertised in
/// AvailableCursorModes. Setting a cursor mode not advertised will cause the
/// screen cast session to be closed." The pointer is embedded when offered,
/// else the portal default (Hidden) applies; monitors and windows as far as
/// AvailableSourceTypes lists either (unreadable: both, as before).
pub(crate) async fn advertised_sources(
    screencast: &ashpd::desktop::screencast::Screencast,
) -> ashpd::desktop::screencast::SelectSourcesOptions {
    use ashpd::desktop::screencast::{CursorMode, SelectSourcesOptions, SourceType};
    let wanted = SourceType::Monitor | SourceType::Window;
    let types = match screencast.available_source_types().await {
        Ok(available) if !(available & wanted).is_empty() => available & wanted,
        _ => wanted,
    };
    let cursor = match screencast.available_cursor_modes().await {
        Ok(modes) if modes.contains(CursorMode::Embedded) => Some(CursorMode::Embedded),
        _ => None,
    };
    SelectSourcesOptions::default()
        .set_cursor_mode(cursor)
        .set_sources(types)
        .set_multiple(false)
}

pub(crate) fn session_path<T: SessionPortal>(session: &Session<T>) -> Result<String, Refusal> {
    serde_json::to_value(session)
        .map_err(failed)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("session handle is not an object path"))
}

impl StartResults {
    pub(crate) fn selected<T: SessionPortal>(
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
}

/// Subscribe before Start so even an immediate response cannot be missed.
/// Only the existing session is started; no second dialog/session or input transport is created.
pub(crate) async fn start<T: SessionPortal>(
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
