//! Remote backends at full speed (package KP3 of the plan
//! `docs/plaene/2026-10-28-verbinden-ist-einrichtung`, analysis G3) against
//! real servers: Cargo builds this directory as the test target
//! `transfer_task`. Every case is ignored; the suite starts the servers,
//! names them in the environment (`support.rs`) and runs
//! `cargo test --test transfer_task -- --ignored --test-threads=1`.
//!
//! - `crash_log.rs`: jobs with the crash log on (`recoverable: true`, as the
//!   GUI, Android and "provide for other programs" run them), which the older
//!   container cases never did (`support::job`). G3-01: a packet download from
//!   SFTP or SMB ran its sink inside the backend's runtime and aborted on the
//!   receipts' nested `block_on`.
//! - `speed.rs`: `speed_task_remote_*` benchmarks of the G5 cases, one
//!   `BENCH <case> files_per_s=<n> mib_per_s=<n>` line per measurement;
//!   with the large-file cases of G3-10 (SFTP with extra SSH connections,
//!   checked by the transfer note) and G3-11 (FTP ranges in a fresh job,
//!   checked byte for byte).
mod crash_log;
mod speed;
mod support;
