//! Why a client registration is refused (FC4, R-SERVER-02, R-SERVER-04). The
//! messages are the texts the clients show; they stay as they were.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegistrationError {
    Full,
    SourceFull,
    InvalidDeviceId,
    IdExhausted,
    /// The device id belongs to another proven key.
    DeviceBound,
    /// The proven key already has its maximum of registrations.
    KeyFull,
    /// The server accepts only clients that log in with their key.
    KeyLoginRequired,
    BindingStateUnavailable,
    /// The proven key already bound its maximum of device ids.
    KeyDevicesFull,
    /// Too many new device bindings from this source; retry later.
    BindingRateLimited,
}

impl RegistrationError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Full => "server client limit reached",
            Self::SourceFull => "server source client limit reached",
            Self::InvalidDeviceId => "invalid or oversized device id",
            Self::IdExhausted => "server client id space exhausted",
            Self::DeviceBound => "device id is bound to another device key",
            Self::KeyFull => "too many connections for this device key",
            Self::BindingStateUnavailable => {
                "cannot persist device key binding; check server state file"
            }
            Self::KeyLoginRequired => {
                "this server requires key login; update Smart Explorer \
                 (the operator can admit older apps with --allow-legacy-clients)"
            }
            Self::KeyDevicesFull => "too many device ids for this device key",
            Self::BindingRateLimited => {
                "too many new device bindings from this network; retry later"
            }
        }
    }
}
