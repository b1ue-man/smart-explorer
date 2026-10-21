//! Measurement cases of the stage `speed` of the batch suite (U55, plan
//! `docs/plaene/2026-10-17-rechte-desktop-tresore/festlegungen.md` § U55 and
//! § U44 M-1 to M-7): the throughput of Smart Explorer per way, compared with
//! the capacity of the same path (`iperf3` for lines, `fio` for drives) and
//! with the plain tools (`sftp`, `curl`, `cp`, `rsync`, `robocopy`, `rclone`).
//!
//! Every case is `#[ignore]`. `native/rdt_task/speed.py` starts the servers
//! and the shaped link (netem on the loopback), then runs
//! `cargo test --test speed_task -- --ignored --test-threads=1 --exact <paths>`
//! with the variables named in `speed_task/support.rs`, `speed_task/remote.rs`
//! and `native/rdt_task/speed_net.py`. A case that misses a variable or a tool
//! prints `SPEED_NOT_RUN <name> <reason>`; every measured case appends one JSON
//! line to `SE_SPEED_REPORT`. Nothing here runs without that stage.
#[path = "speed_task/data.rs"]
mod data;
#[path = "speed_task/reference.rs"]
mod reference;
#[path = "speed_task/remote.rs"]
mod remote;
#[path = "speed_task/run.rs"]
mod run;
#[path = "speed_task/support.rs"]
mod support;
#[path = "speed_task/tools.rs"]
mod tools;

#[cfg(unix)]
#[path = "speed_task/cli.rs"]
mod cli;
#[cfg(unix)]
#[path = "speed_task/compress.rs"]
mod compress;
#[cfg(unix)]
#[path = "speed_task/limiter.rs"]
mod limiter;
#[cfg(unix)]
#[path = "speed_task/net_large.rs"]
mod net_large;
#[cfg(unix)]
#[path = "speed_task/net_small.rs"]
mod net_small;
#[cfg(unix)]
#[path = "speed_task/ramp.rs"]
mod ramp;
#[cfg(unix)]
#[path = "speed_task/raw.rs"]
mod raw;
#[cfg(unix)]
#[path = "speed_task/room.rs"]
mod room;

#[cfg(target_os = "linux")]
#[path = "speed_task/local.rs"]
mod local;
#[cfg(windows)]
#[path = "speed_task/windows.rs"]
mod windows;
