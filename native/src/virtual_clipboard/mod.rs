#[path = "os/windows.rs"]
mod imp;
#[path = "os/remote.rs"]
mod remote;

pub use imp::*;
pub use remote::set_remote_clipboard;
