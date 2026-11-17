//! Resumable HTTP downloads without a total time limit (user rule
//! 2026-11-17): the update payloads and the Android package
//! (`updater`), the FFmpeg and OpenH264 libraries of the screen viewer
//! (`screen`) and the Dokany installer (`mount`).
#[path = "core/decision.rs"]
mod decision;
#[path = "os/shared/fetch.rs"]
mod fetch;

pub(crate) use fetch::{fetch, Bound, Source};
