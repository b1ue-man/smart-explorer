//! Portable P5 wire and decision types. Platform launch targets never enter here.
use crate::screen::core::app_icon::IconRef;
use serde::{Deserialize, Serialize};
use std::{fmt, io};

pub(crate) fn invalid(reason: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason.into())
}

macro_rules! identifier {
    ($name:ident, $max:expr) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub(crate) struct $name(String);
        impl $name {
            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = io::Error;
            fn try_from(text: String) -> io::Result<Self> {
                if text.is_empty() || text.len() > $max || text.chars().any(char::is_control) {
                    return Err(invalid(concat!("invalid ", stringify!($name))));
                }
                Ok(Self(text))
            }
        }
        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                out.write_str(&self.0)
            }
        }
    };
}
identifier!(AppId, 1024);
identifier!(DeviceId, 1024);
identifier!(UserId, 1024);
identifier!(SessionId, 1024);
identifier!(RequestId, 128);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostSession {
    pub user_id: UserId,
    /// Includes the OS login incarnation, not just a reusable numeric session ID.
    pub session_id: SessionId,
    pub interactive: bool,
    pub unlocked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppKind {
    WindowsShell,
    LinuxDesktopEntry,
    AndroidLauncher,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogApp {
    pub app_id: AppId,
    pub name: String,
    pub kind: AppKind,
    /// P3 identity may differ from launch identity (Android component/profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<IconRef>,
    pub can_launch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogSnapshot {
    pub version: u8,
    pub device_id: DeviceId,
    pub session: HostSession,
    /// SHA-256 of session, identities and actual installed launch definitions.
    pub revision: String,
    /// E4's app-launch session constraint uses this catalog-scoped value. The
    /// real OS login remains `session.session_id` and `LaunchRequest.session_id`.
    /// Installing/rebinding an app or logging out cannot inherit an old grant.
    pub authorization_scope: SessionId,
    pub apps: Vec<CatalogApp>,
    pub limits: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaunchRequest {
    pub request_id: RequestId,
    pub target_device_id: DeviceId,
    pub app_id: AppId,
    pub user_id: UserId,
    pub session_id: SessionId,
    pub catalog_revision: String,
}

#[derive(Clone, Debug)]
pub(crate) struct LaunchFacts {
    pub request: LaunchRequest,
    pub catalog: CatalogSnapshot,
    pub app: CatalogApp,
    pub authorization_scope: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LaunchState {
    /// Durable admission preceded OS dispatch. A crash/bridge timeout here must
    /// never be retried automatically: the OS may already have accepted it.
    OutcomeUnknown,
    StartedWindowPending,
    Started,
    Activated,
    ConsentRequired,
    Denied,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowBinding {
    pub app_id: AppId,
    pub user_id: UserId,
    pub session_id: SessionId,
    /// P4's generation-bound identity, never a raw reusable HWND/XID alone.
    pub window_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaunchReceipt {
    pub request: LaunchRequest,
    pub launch_id: String,
    /// Host-generated admission phase; stale local bridge replies cannot settle
    /// a later phase of the same stable remote request.
    #[serde(default)]
    pub dispatch_id: String,
    pub state: LaunchState,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowBinding>,
}

#[derive(Clone, Debug)]
pub(crate) enum LaunchDenial {
    ConsentRequired(String),
    Denied(String),
    Unsupported(String),
}
impl LaunchDenial {
    pub(crate) fn parts(self) -> (LaunchState, String) {
        match self {
            Self::ConsentRequired(s) => (LaunchState::ConsentRequired, s),
            Self::Denied(s) => (LaunchState::Denied, s),
            Self::Unsupported(s) => (LaunchState::Unsupported, s),
        }
    }
}
