#!/usr/bin/env bash
# Single RV1 task-level entrypoint. Remote CI only; never run on the workstation.
set -Eeuo pipefail
[[ "${GITHUB_ACTIONS:-}" == true || "${SMART_EXPLORER_REMOTE_RUNNER:-}" == 1 ]] || {
  echo 'RV1 acceptance runs only on the configured remote runner.' >&2
  exit 2
}
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[[ $# -ge 2 && $1 == --stage ]] || { echo 'Usage: native/test-review-task.sh --stage native|android-build|device [arguments]' >&2; exit 2; }
stage=$2
shift 2
[[ "${CANDIDATE_SHA:-}" =~ ^[0-9a-f]{40}$ && "$(git -C "$root" rev-parse HEAD)" == "$CANDIDATE_SHA" && "${GITHUB_SHA:-$CANDIDATE_SHA}" == "$CANDIDATE_SHA" ]] || {
  echo 'Ref, checkout and full candidate SHA must agree.' >&2
  exit 2
}
export SMART_EXPLORER_TASK_LOG_ROOT="${SMART_EXPLORER_TASK_LOG_ROOT:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/review-task-logs}"
mkdir -p "$SMART_EXPLORER_TASK_LOG_ROOT"
printf '%s\n' "$CANDIDATE_SHA" >"$SMART_EXPLORER_TASK_LOG_ROOT/candidate.txt"
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=1 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TERM_COLOR=never
case "$stage" in
  native) exec python3 "$root/native/review-task-native.py" "$@" ;;
  android-build) exec python3 "$root/native/review-task-native.py" --android-build "$@" ;;
  device) exec python3 "$root/native/review-task-device.py" "$@" ;;
  *) echo "Unknown RV1 stage: $stage" >&2; exit 2 ;;
esac
