#[path = "windows/platform.rs"]
mod platform;
#[path = "shared/platform_helpers.rs"]
pub(in crate::app) mod shared_platform_helpers;
#[path = "windows/shell_menus.rs"]
mod shell_menus;
#[path = "windows/watchers.rs"]
mod watchers;

pub(in crate::app) use platform::*;
pub(in crate::app) use shared_platform_helpers::*;
