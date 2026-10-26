//! Pure AccessV2 target conditions. Identities are host-resolved stable IDs;
//! they are never executable paths, display names or a request to switch users.
use crate::share::access_grant::Capability;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

macro_rules! identity {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            pub fn parse(value: &str) -> Result<Self, String> {
                if value.trim().is_empty() || value.chars().any(misleading_identity) {
                    return Err(
                        concat!(stringify!($name), " must be a nonempty plain identity").into(),
                    );
                }
                Ok(Self(value.into()))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(value: String) -> Result<Self, String> {
                Self::parse(&value)
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
fn misleading_identity(character: char) -> bool {
    character.is_control()
        || matches!(character,
        '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
        | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}
identity!(AppId);
identity!(SessionId);
identity!(UserId);
identity!(NodeId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct TcpPort(u16);
impl TcpPort {
    pub fn get(self) -> u16 {
        self.0
    }
}
impl TryFrom<u16> for TcpPort {
    type Error = String;
    fn try_from(value: u16) -> Result<Self, String> {
        if value == 0 {
            Err("TCP destination port must be nonzero".into())
        } else {
            Ok(Self(value))
        }
    }
}
impl From<TcpPort> for u16 {
    fn from(value: TcpPort) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TcpDirection {
    PeerToHost,
    HostToPeer,
}

/// DNS names have an explicit IP allowlist. The socket adapter must resolve
/// freshly, check each selected IP here and connect to that checked IP itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TcpHost {
    #[default]
    Loopback,
    Ip {
        address: IpAddr,
    },
    Dns {
        name: String,
        addresses: Vec<IpAddr>,
    },
}
impl TcpHost {
    fn validate(&self) -> Result<(), String> {
        let plain_ip = |ip: &IpAddr| !ip.is_unspecified() && !ip.is_multicast();
        match self {
            Self::Loopback => Ok(()),
            Self::Ip { address } if plain_ip(address) => Ok(()),
            Self::Dns { name, addresses }
                if valid_dns_name(name)
                    && !addresses.is_empty()
                    && addresses.iter().all(plain_ip) =>
            {
                Ok(())
            }
            _ => Err("TCP host needs an exact IP or a DNS name with explicit allowed IPs".into()),
        }
    }
    fn allows(&self, requested: &TcpHost, resolved: IpAddr) -> bool {
        match (self, requested) {
            (Self::Loopback, Self::Loopback) => resolved.is_loopback(),
            (Self::Ip { address }, Self::Ip { address: requested }) => {
                address == requested && *address == resolved
            }
            (
                Self::Dns { name, addresses },
                Self::Dns {
                    name: requested, ..
                },
            ) => name.eq_ignore_ascii_case(requested) && addresses.contains(&resolved),
            _ => false,
        }
    }
    fn covers(&self, old: &Self) -> bool {
        match (self, old) {
            (
                Self::Dns { name, addresses },
                Self::Dns {
                    name: before,
                    addresses: ips,
                },
            ) => name.eq_ignore_ascii_case(before) && contains_all(addresses, ips),
            _ => self == old,
        }
    }
}
fn valid_dns_name(name: &str) -> bool {
    name.len() <= 253
        && name.parse::<IpAddr>().is_err()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginOperation {
    Login,
    Unlock,
    SecureDesktop,
    UserSession,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppLaunchConstraints {
    pub app_ids: Vec<AppId>,
    pub sessions: Vec<SessionId>,
    pub users: Vec<UserId>,
    #[serde(default)]
    pub valid_until: Option<i64>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginAccessConstraints {
    pub sessions: Vec<SessionId>,
    pub users: Vec<UserId>,
    pub operations: Vec<LoginOperation>,
    #[serde(default)]
    pub valid_until: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TcpForwardRule {
    pub initiator: NodeId,
    pub target: NodeId,
    pub direction: TcpDirection,
    #[serde(default)]
    pub host: TcpHost,
    pub ports: Vec<TcpPort>,
    #[serde(default)]
    pub valid_until: Option<i64>,
}
impl TcpForwardRule {
    fn covers(&self, old: &Self) -> bool {
        self.initiator == old.initiator
            && self.target == old.target
            && self.direction == old.direction
            && self.host.covers(&old.host)
            && contains_all(&self.ports, &old.ports)
            && !earlier(self.valid_until, old.valid_until)
    }
    fn allows(&self, request: &TcpForwardRequest, now: i64) -> bool {
        self.initiator == request.initiator
            && self.target == request.target
            && self.direction == request.direction
            && self.ports.contains(&request.port)
            && self.host.allows(&request.host, request.resolved_ip)
            && !ended(self.valid_until, now)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_launch: Option<AppLaunchConstraints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_access: Option<LoginAccessConstraints>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tcp_forward: Vec<TcpForwardRule>,
}
impl AccessConstraints {
    pub fn is_empty(&self) -> bool {
        self.app_launch.is_none() && self.login_access.is_none() && self.tcp_forward.is_empty()
    }
    pub fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::AppLaunch => self.app_launch.is_some(),
            Capability::LoginAccess => self.login_access.is_some(),
            Capability::TcpForward => !self.tcp_forward.is_empty(),
            _ => true,
        }
    }
    /// Whether any configured target of the capability is still reachable
    /// at this time; it is a display fact, never an operation authorization.
    pub fn available_at(&self, capability: Capability, now: i64) -> bool {
        match capability {
            Capability::AppLaunch => self
                .app_launch
                .as_ref()
                .is_some_and(|app| !ended(app.valid_until, now)),
            Capability::LoginAccess => self
                .login_access
                .as_ref()
                .is_some_and(|login| !ended(login.valid_until, now)),
            Capability::TcpForward => self
                .tcp_forward
                .iter()
                .any(|rule| !ended(rule.valid_until, now)),
            _ => true,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(app) = &self.app_launch {
            if app.app_ids.is_empty() || app.sessions.is_empty() || app.users.is_empty() {
                return Err("app_launch needs nonempty app, session and user lists".into());
            }
            valid_end(app.valid_until)?;
        }
        if let Some(login) = &self.login_access {
            if login.sessions.is_empty() || login.users.is_empty() || login.operations.is_empty() {
                return Err("login_access needs nonempty session, user and operation lists".into());
            }
            valid_end(login.valid_until)?;
        }
        for rule in &self.tcp_forward {
            if rule.initiator == rule.target || rule.ports.is_empty() {
                return Err("tcp_forward needs distinct endpoint IDs and explicit ports".into());
            }
            rule.host.validate()?;
            valid_end(rule.valid_until)?;
        }
        Ok(())
    }
    pub fn allows(&self, request: &AccessRequest, now: i64) -> bool {
        match request {
            AccessRequest::AppLaunch(request) => self.app_launch.as_ref().is_some_and(|app| {
                app.app_ids.contains(&request.app_id)
                    && app.sessions.contains(&request.session)
                    && app.users.contains(&request.user)
                    && !ended(app.valid_until, now)
            }),
            AccessRequest::LoginAccess(request) => {
                self.login_access.as_ref().is_some_and(|login| {
                    login.users.contains(&request.user)
                        && login.sessions.contains(&request.session)
                        && login.operations.contains(&request.operation)
                        && !ended(login.valid_until, now)
                })
            }
            AccessRequest::TcpForward(request) => self
                .tcp_forward
                .iter()
                .any(|rule| rule.allows(request, now)),
        }
    }
    /// Conservative set containment, shared by all persisted/runtime callers.
    pub fn narrows(&self, old: &Self, capability: Capability) -> bool {
        match capability {
            Capability::AppLaunch => old.app_launch.as_ref().is_some_and(|old| {
                self.app_launch.as_ref().is_none_or(|new| {
                    !contains_all(&new.app_ids, &old.app_ids)
                        || !contains_all(&new.sessions, &old.sessions)
                        || !contains_all(&new.users, &old.users)
                        || earlier(new.valid_until, old.valid_until)
                })
            }),
            Capability::LoginAccess => old.login_access.as_ref().is_some_and(|old| {
                self.login_access.as_ref().is_none_or(|new| {
                    !contains_all(&new.sessions, &old.sessions)
                        || !contains_all(&new.users, &old.users)
                        || !contains_all(&new.operations, &old.operations)
                        || earlier(new.valid_until, old.valid_until)
                })
            }),
            Capability::TcpForward => old
                .tcp_forward
                .iter()
                .any(|old| !self.tcp_forward.iter().any(|new| new.covers(old))),
            _ => false,
        }
    }
    /// The end of the currently matching condition; TCP alternatives retain
    /// their separate ends, and the latest matching alternative may authorize.
    pub fn end_for(&self, request: &AccessRequest, now: i64) -> Option<i64> {
        match request {
            AccessRequest::AppLaunch(_) => self.app_launch.as_ref().and_then(|app| app.valid_until),
            AccessRequest::LoginAccess(_) => self
                .login_access
                .as_ref()
                .and_then(|login| login.valid_until),
            AccessRequest::TcpForward(request) => {
                let matching: Vec<_> = self
                    .tcp_forward
                    .iter()
                    .filter(|rule| rule.allows(request, now))
                    .collect();
                if matching.iter().any(|rule| rule.valid_until.is_none()) {
                    None
                } else {
                    matching.iter().filter_map(|rule| rule.valid_until).max()
                }
            }
        }
    }
    pub fn deadlines(&self) -> impl Iterator<Item = i64> + '_ {
        self.app_launch
            .iter()
            .filter_map(|app| app.valid_until)
            .chain(
                self.login_access
                    .iter()
                    .filter_map(|login| login.valid_until),
            )
            .chain(self.tcp_forward.iter().filter_map(|rule| rule.valid_until))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppLaunchRequest {
    pub app_id: AppId,
    pub session: SessionId,
    pub user: UserId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginAccessRequest {
    pub session: SessionId,
    pub user: UserId,
    pub operation: LoginOperation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TcpForwardRequest {
    pub initiator: NodeId,
    pub target: NodeId,
    pub direction: TcpDirection,
    pub host: TcpHost,
    pub resolved_ip: IpAddr,
    pub port: TcpPort,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "capability", content = "target", rename_all = "snake_case")]
pub enum AccessRequest {
    AppLaunch(AppLaunchRequest),
    LoginAccess(LoginAccessRequest),
    TcpForward(TcpForwardRequest),
}
impl AccessRequest {
    pub fn capability(&self) -> Capability {
        match self {
            Self::AppLaunch(_) => Capability::AppLaunch,
            Self::LoginAccess(_) => Capability::LoginAccess,
            Self::TcpForward(_) => Capability::TcpForward,
        }
    }
}
fn contains_all<T: PartialEq>(new: &[T], old: &[T]) -> bool {
    old.iter().all(|item| new.contains(item))
}
pub(crate) fn earlier(new: Option<i64>, old: Option<i64>) -> bool {
    match (old, new) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(old), Some(new)) => new < old,
    }
}
fn ended(end: Option<i64>, now: i64) -> bool {
    end.is_some_and(|end| now >= end)
}
fn valid_end(end: Option<i64>) -> Result<(), String> {
    if end.is_some_and(|end| end <= 0) {
        Err("condition expiry must be after 1970".into())
    } else {
        Ok(())
    }
}
