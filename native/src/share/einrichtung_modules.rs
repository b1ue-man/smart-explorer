//! Modules of the plan 2026-10-28 „Verbinden ist die Einrichtung“ (work
//! packages WP1, host side, and WP2, viewer side): `share/mod.rs` declares
//! this file once and re-exports its modules, like `access_modules.rs` for
//! strand S1.
#[path = "os/shared/auto_forward.rs"]
pub(crate) mod auto_forward;
#[path = "core/device_address.rs"]
pub(crate) mod device_address;
#[cfg(test)]
#[path = "core/einrichtung_host_task_tests.rs"]
mod einrichtung_host_task_tests;
#[cfg(test)]
#[path = "core/einrichtung_link_task_tests.rs"]
mod einrichtung_link_task_tests;
#[cfg(test)]
#[path = "core/einrichtung_review_task_tests.rs"]
mod einrichtung_review_task_tests;
#[path = "core/loopback_services.rs"]
pub(crate) mod loopback_services;
/// OS facade of TCP socket facts (design 9.1 `crate::share::os_sockets`).
#[cfg(windows)]
#[path = "os/windows/tcp_table.rs"]
pub(crate) mod os_sockets;
/// OS facade of TCP socket facts; Android reports its `/proc/net` limit.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[path = "os/linux_os/proc_net_tcp.rs"]
pub(crate) mod os_sockets;
/// Device feed per present device (WP2, design 5).
#[path = "os/shared/peer_feed.rs"]
pub(crate) mod peer_feed;
#[path = "core/peer_select.rs"]
pub(crate) mod peer_select;
