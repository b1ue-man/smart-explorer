//! P4/V6: session-bound window families. The screen grant still authorizes
//! session-wide input; HWND/XID focus checks do not provide app isolation.
pub(crate) mod core;
pub(crate) mod os;

pub(crate) use core::host::{HostSession, WindowAdapter, WindowAuthority};
pub(crate) use core::model::*;
