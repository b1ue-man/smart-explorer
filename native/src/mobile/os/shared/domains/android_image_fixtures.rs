//! Explicit instrumentation facade. No generic API method routes here.
use crate::screen::{core::pixel_convert::FrameSize, os::android_capture_fixture as capture};
fn error(error: std::io::Error) -> String {
    error.to_string()
}
pub fn android_fixture_capture_open(
    width: u32,
    height: u32,
    output_width: u32,
    output_height: u32,
    kind: u32,
    heap: u64,
    decoder: bool,
) -> Result<String, String> {
    capture::open(
        FrameSize::new(width, height),
        FrameSize::new(output_width, output_height),
        kind,
        heap,
        decoder,
    )
    .map(|value| value.to_string())
    .map_err(error)
}
pub fn android_fixture_capture_state(session: u64) -> Result<String, String> {
    capture::state(session)
        .map(|value| value.to_string())
        .map_err(error)
}
pub fn android_fixture_capture_consume_still(
    session: u64,
    generation: u64,
) -> Result<String, String> {
    capture::consume_still(session, generation)
        .map(|value| value.to_string())
        .map_err(error)
}
pub fn android_fixture_capture_pull_address(session: u64) -> Result<(usize, usize), String> {
    capture::pull_address(session).map_err(error)
}
pub fn android_fixture_capture_release_reader(session: u64) -> Result<(), String> {
    capture::release_reader(session).map_err(error)
}
pub fn android_fixture_capture_release_viewer(session: u64) -> Result<(), String> {
    capture::release_viewer(session).map_err(error)
}
pub fn android_fixture_capture_retire(session: u64) -> Result<(), String> {
    capture::retire(session).map_err(error)
}
pub fn android_fixture_capture_finish(session: u64) -> Result<(), String> {
    capture::finish(session).map_err(error)
}
pub fn android_fixture_login_open(
    owner: u64,
    heap: u64,
    width: u32,
    height: u32,
) -> Result<u64, String> {
    super::login_image_fixture::open(owner, heap, width, height).map_err(error)
}
pub fn android_fixture_login_enqueue(session: u64, frame: u64, pattern: u32) -> Result<(), String> {
    super::login_image_fixture::enqueue(session, frame, pattern).map_err(error)
}
pub fn android_fixture_login_state(session: u64) -> Result<String, String> {
    super::login_image_fixture::state(session)
        .map(|value| value.to_string())
        .map_err(error)
}
pub fn android_fixture_login_finish(session: u64) -> Result<(), String> {
    super::login_image_fixture::finish(session).map_err(error)
}
