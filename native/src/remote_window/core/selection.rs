//! Platform-selected sources carry capabilities, never arbitrary app-window rights.
use super::model::CaptureGeometry;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PortalInputPath {
    None,
    Notify,
    Eis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortalSelection {
    pub session: String,
    pub node_id: u32,
    pub pipewire_serial: Option<u64>,
    pub logical_geometry: CaptureGeometry,
    pub pointer_granted: bool,
    pub keyboard_granted: bool,
    pub touch_granted: bool,
    pub input_path: PortalInputPath,
}

impl PortalSelection {
    /// Facts must come from this session's successful portal Start reply. A
    /// restore token is not a grant. Without serial keep the same open PipeWire
    /// remote/stream; a lost stream requires a fresh portal session and identity.
    pub(crate) fn validate_stream(
        &self,
        session: &str,
        node: u32,
        serial: Option<u64>,
        same_open_stream: bool,
    ) -> io::Result<()> {
        let serial_matches = match self.pipewire_serial {
            Some(expected) => serial == Some(expected),
            None => same_open_stream && serial.is_none(),
        };
        if self.session.is_empty()
            || session != self.session
            || node != self.node_id
            || !serial_matches
            || !self.logical_geometry.valid()
        {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "portal_stream_identity_changed",
            ));
        }
        Ok(())
    }

    pub(crate) fn admit_input(
        &self,
        path: PortalInputPath,
        pointer: bool,
        keyboard: bool,
        touch: bool,
    ) -> io::Result<()> {
        if path == PortalInputPath::None
            || path != self.input_path
            || (pointer && !self.pointer_granted)
            || (keyboard && !self.keyboard_granted)
            || (touch && !self.touch_granted)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "portal_input_not_granted",
            ));
        }
        Ok(())
    }

    /// Portal absolute coordinates are logical stream coordinates, never global
    /// desktop positions or unscaled PipeWire pixel positions.
    pub(crate) fn logical_pointer(&self, x: u16, y: u16) -> io::Result<(f64, f64)> {
        if !self.logical_geometry.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "portal_logical_size_unknown",
            ));
        }
        Ok((
            f64::from(x) * f64::from(self.logical_geometry.width.saturating_sub(1)) / 65_535.0,
            f64::from(y) * f64::from(self.logical_geometry.height.saturating_sub(1)) / 65_535.0,
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AndroidSelection {
    /// Fresh projection generation supplied by the host bridge.
    pub projection: String,
    /// Only when the platform actually reports/verifies the chosen package.
    pub system_chosen_package: Option<String>,
    pub single_app: bool,
}

impl AndroidSelection {
    pub(crate) fn validate(
        &self,
        current_projection: &str,
        requested_package: Option<&str>,
    ) -> io::Result<()> {
        if !self.single_app || self.projection.is_empty() || self.projection != current_projection {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "android_selected_app_unavailable",
            ));
        }
        if requested_package.is_some() && requested_package != self.system_chosen_package.as_deref()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "android_system_selection_required",
            ));
        }
        Ok(())
    }
}
