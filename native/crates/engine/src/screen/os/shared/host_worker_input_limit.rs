//! How long the daemon waits for a host process's answer to input, and
//! that this wait stays strict (review 2026-11-19, Gemini 11.3, Gemini 3/11).
use crate::screen::core::worker_family::FamilyCommand;
use crate::screen::core::worker_wire::WorkerRequest;

/// How long a host process may take for input events; defined in the core
/// (`worker_limits::input_limit`) so the release keeper uses the same time.
pub(super) use crate::screen::core::worker_limits::input_limit;

/// Input waits for its answer within its limit, even a longer one: a lost
/// answer ends the process (`WorkerReader::receive` with `strict`), the
/// picture stream's messages never keep it waiting.
pub(super) fn strict_answer(request: &WorkerRequest) -> bool {
    matches!(
        request,
        WorkerRequest::Input(_)
            | WorkerRequest::Release(_)
            | WorkerRequest::Family(FamilyCommand::Input(_))
    )
}
