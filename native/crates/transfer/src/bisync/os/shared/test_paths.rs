//! Path spelling of the bisync tests: the forward-slash form of a local path.
//! The engine's link tests use it too, so it lives outside the test-only tree
//! (plan docs/plaene/2026-11-01-idealzustand, D13).
use std::path::Path;

#[doc(hidden)]
pub fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
