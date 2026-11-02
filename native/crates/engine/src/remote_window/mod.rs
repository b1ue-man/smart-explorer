//! P4/V6: session-bound window families. The screen grant still authorizes
//! session-wide input; HWND/XID focus checks do not provide app isolation.
pub mod core;
pub mod os;

pub use core::host::{HostSession, WindowAdapter, WindowAuthority};
pub use core::model::*;
