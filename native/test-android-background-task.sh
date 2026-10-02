#!/usr/bin/env bash
# Single task-level suite for the Android background and storage analysis batch
# (docs/superpowers/plans/2026-10-01-android-hintergrund-analyse/). Run this one
# checked-in entrypoint on the remote runner with an outer timeout of at least
# 30 minutes. The Android device part runs through android/test-android-task.sh
# (jobs android-host, desktop-bins, android-build, device of the workflow).
#
#   --check   narrow compile check while the batch is being built: the batch
#             gates only (rustfmt of every changed file, clippy for the host and
#             the Windows target over all targets; diagnostics on changed lines
#             are reported). No tests run.
#   (none)    the complete suite: gating batch gates, every milestone test
#             (`android_background_task_`), the tests of every module the batch
#             touched, the Share server's own tests (idle keepalive, presence
#             bundling, relay ping) and, on Linux, the Share Room end-to-end run
#             with the real CLI and server.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-android-background-task.sh [--check] [--bounded|--direct]" >&2
    echo "  --check    batch gates only (compile, rustfmt, clippy on changed lines)" >&2
    echo "  --bounded  use native/run-task-memory-bounded.sh (default for local users)" >&2
    echo "  --direct   rely on the remote runner's resource controls" >&2
}

execution_mode=bounded
suite_mode=suite
for argument in "$@"; do
    case "$argument" in
        --check) suite_mode=check ;;
        --bounded) execution_mode=bounded ;;
        --direct) execution_mode=direct ;;
        *) usage; exit 2 ;;
    esac
done

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) platform=windows ;;
    *) platform=linux ;;
esac

report_failure() {
    local status=$?
    echo "android background task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-android-background-task.XXXXXX")"
fi
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -f "$suite_tmp/batch-ranges.txt" "$suite_tmp"/clippy-*.log
        rmdir "$suite_tmp" 2>/dev/null || true
    else
        echo "android background task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in awk cargo comm git grep mktemp rustfmt sed sort tee timeout tr uname; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "$command_name is required" >&2
        exit 1
    }
done
if [[ "$execution_mode" == bounded && "$platform" == linux ]]; then
    test -x "$repo_root/native/run-task-memory-bounded.sh" || {
        echo "task memory wrapper is missing or not executable" >&2
        exit 1
    }
fi

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_TERM_COLOR=never

run_task() {
    if [[ "$execution_mode" == bounded && "$platform" == linux ]]; then
        "$repo_root/native/run-task-memory-bounded.sh" "$@"
    else
        "$@"
    fi
}

echo "android background task suite: batch gates ($platform, $suite_mode)"
# The crate as a whole carries older formatting and dead-code drift outside
# this batch (docs/TODO.md, H1), so the gates cover exactly what this batch
# touched: every changed file is format-checked on its own through rustfmt's
# stdin mode, and clippy runs over all targets of the crate, but only
# diagnostics on lines this batch changed fail. A compile error anywhere still
# fails. The batch base is the last commit before the batch.
batch_base=fe21721031ed4612bb5e214e8915dd6fcf1dc4b7
if ! git -C "$repo_root" cat-file -e "${batch_base}^{commit}" 2>/dev/null; then
    git -C "$repo_root" fetch --quiet --depth=1 origin "$batch_base"
fi
mapfile -t batch_files < <(
    git -C "$repo_root" diff --name-only --diff-filter=AM "$batch_base" HEAD -- \
        'native/src/*.rs' 'native/tests/*.rs' 'share-server/src/*.rs'
)
if [[ "${#batch_files[@]}" -eq 0 ]]; then
    echo "no batch source files found relative to $batch_base" >&2
    exit 1
fi
# In the complete suite a failing stage is recorded and the next one still
# runs, so one remote run reports every problem; the suite fails at its end.
# The check mode stops at the first failure.
failed_stages=()
stage_failed() {
    failed_stages+=("$1")
    echo "android background task suite: FAILED: $1" >&2
    if [[ "$suite_mode" == check ]]; then
        exit 1
    fi
}
finish_suite() {
    if [[ "${#failed_stages[@]}" -ne 0 ]]; then
        printf 'android background task suite: failed stage: %s\n' "${failed_stages[@]}" >&2
        exit 1
    fi
    suite_succeeded=true
    echo "$1"
}

format_failures=0
for batch_file in "${batch_files[@]}"; do
    format_diff="$(rustfmt --check --color never --edition 2021 < "$repo_root/$batch_file")"
    if [[ -n "$format_diff" ]]; then
        printf '%s\n' "$format_diff" | sed "s#<stdin>#$batch_file#"
        format_failures=$((format_failures + 1))
    fi
done
if [[ "$format_failures" -ne 0 ]]; then
    stage_failed "rustfmt: $format_failures batch source files are not rustfmt-clean"
else
    echo "android background task suite: ${#batch_files[@]} batch source files are rustfmt-clean"
fi

batch_ranges="$suite_tmp/batch-ranges.txt"
: > "$batch_ranges"
for batch_file in "${batch_files[@]}"; do
    [[ "$batch_file" == native/* ]] || continue
    git -C "$repo_root" diff -U0 "$batch_base" HEAD -- "$batch_file" | awk -v file="${batch_file#native/}" '
        /^@@ / {
            split($3, plus, ",")
            start = substr(plus[1], 2) + 0
            count = (length(plus) > 1) ? plus[2] + 0 : 1
            if (count > 0) {
                print file, start, start + count - 1
            }
        }' >> "$batch_ranges"
done
batch_diagnostics() {
    local log=$1
    # Cargo prints Windows paths with backslashes; the ranges use slashes.
    { tr '\\' '/' < "$log" | grep -E '^(src|tests)/[^:]+:[0-9]+:[0-9]+: (warning|error)' || true; } |
        sort -u | awk -F: -v ranges="$batch_ranges" '
        BEGIN {
            while ((getline line < ranges) > 0) {
                split(line, range, " ")
                n++
                range_file[n] = range[1]
                range_start[n] = range[2]
                range_end[n] = range[3]
            }
        }
        {
            for (i = 1; i <= n; i++) {
                if ($1 == range_file[i] && $2 >= range_start[i] && $2 <= range_end[i]) {
                    print
                    break
                }
            }
        }'
}
clippy_targets=(host)
if [[ "$platform" == linux ]]; then
    if ! rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$'; then
        echo "x86_64-pc-windows-gnu target is required on Linux" >&2
        exit 1
    fi
    command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || {
        echo "x86_64-w64-mingw32-gcc is required on Linux" >&2
        exit 1
    }
    clippy_targets+=(x86_64-pc-windows-gnu)
fi
for clippy_target in "${clippy_targets[@]}"; do
    clippy_log="$suite_tmp/clippy-$clippy_target.log"
    target_arguments=()
    if [[ "$clippy_target" != host ]]; then
        target_arguments=(--target "$clippy_target")
    fi
    if ! (
        cd "$repo_root/native"
        run_task cargo clippy --locked "${target_arguments[@]}" --all-targets \
            --message-format short
    ) 2>&1 | tee "$clippy_log"; then
        stage_failed "clippy ($clippy_target) did not complete"
        continue
    fi
    batch_diagnostic_lines="$(batch_diagnostics "$clippy_log")"
    if [[ -n "$batch_diagnostic_lines" ]]; then
        printf '%s\n' "$batch_diagnostic_lines" >&2
        if [[ "$suite_mode" == check ]]; then
            # While the batch is built, stubs and not yet wired entry points
            # are expected to be unused; the check reports them and goes on.
            echo "clippy ($clippy_target): diagnostics on changed lines (reported, not gating in --check)" >&2
            continue
        fi
        stage_failed "clippy ($clippy_target) reported diagnostics on lines this batch changed"
        continue
    fi
    echo "android background task suite: clippy ($clippy_target) is clean on the lines this batch changed"
done

if [[ "$suite_mode" == check ]]; then
    suite_succeeded=true
    echo "android background task suite: batch gates passed (check)"
    exit 0
fi

# ---------------------------------------------------------------------------
# Milestone tests: every `android_background_task_` test of the library. The
# compiled list is the truth of what runs; the source is compared against it
# so a test module that silently stopped compiling (a lost `mod` line, a
# wrong cfg) fails the suite instead of shrinking it.
milestone_log="$suite_tmp/milestones.log"
echo "android background task suite: milestone tests ($platform)"
# A hanging test must not hold the job until its timeout: the stage gets its
# own limit and the test that never finished is named.
report_hung_test() {
    local log=$1 started finished
    started="$(grep -oE '^test [^ ]+ \.\.\. ' "$log" | awk '{print $2}' | tail -n 1 || true)"
    finished="$(grep -E "^test ${started//./\\.} \.\.\. (ok|FAILED|ignored)" "$log" || true)"
    if [[ -n "$started" && -z "$finished" ]]; then
        echo "android background task suite: test did not finish: $started" >&2
    fi
}
if ! (
    cd "$repo_root/native"
    run_task timeout --kill-after=60s 2700 cargo test --locked --lib android_background_task_ -- --test-threads=1
) 2>&1 | tee "$milestone_log"; then
    report_hung_test "$milestone_log"
    stage_failed "milestone tests"
fi
passed_line="$(grep -E '^test result: ok\. [0-9]+ passed; 0 failed' "$milestone_log" | head -n 1 || true)"
mapfile -t source_tests < <(
    grep -rhoE 'fn android_background_task_[A-Za-z0-9_]+' "$repo_root/native/src" |
        sed 's/^fn //' | sort -u
)
mapfile -t compiled_tests < <(
    { grep -oE '^test [^ ]+ \.\.\. (ok|ignored|FAILED)' "$milestone_log" || true; } |
        awk '{print $2}' | sed 's/.*:://' | sort -u
)
mapfile -t missing_tests < <(comm -23 <(printf '%s\n' "${source_tests[@]}") \
    <(printf '%s\n' "${compiled_tests[@]}"))

# The `cfg` attributes in the attribute block directly above `fn <name>`.
test_cfgs() {
    awk -v name="$2" '
        { lines[NR] = $0 }
        $0 ~ ("fn " name "[(<]") { found = NR; exit }
        END {
            for (i = found - 1; found && i > 0; i--) {
                if (lines[i] !~ /^[[:space:]]*(#\[|\/\/)/) break
                if (lines[i] ~ /#\[cfg\(/) printf "%s ", lines[i]
            }
        }' "$1"
}

# Whether those attributes leave the test out on this platform.
cfg_excludes_platform() {
    case "$platform" in
        windows) grep -Eq 'cfg\(((all|any)\()?(unix|target_family = "unix"|target_os = "(linux|android)"|not\(windows\))' <<<"$1" ;;
        linux) grep -Eq 'cfg\(((all|any)\()?(windows|target_family = "windows"|target_os = "windows"|not\(unix\))' <<<"$1" ;;
    esac
}

unexpected=()
for name in "${missing_tests[@]}"; do
    [[ -z "$name" ]] && continue
    defined_in="$(grep -rlE "fn ${name}\b" "$repo_root/native/src" | head -n 1)"
    if cfg_excludes_platform "$(test_cfgs "$defined_in" "$name")"; then
        continue
    fi
    case "$platform:$defined_in" in
        # Windows-only modules and Windows adapters do not exist on Linux.
        linux:*/virtual_clipboard/*|linux:*/dragout/*|linux:*/os/windows*|linux:*/windows/*) ;;
        # Unix adapters do not exist on Windows.
        windows:*/os/linux_os*|windows:*/os/unix*|windows:*/linux_os/*|windows:*/mobile/*) ;;
        # The app trash and its protected areas exist for Android volumes; their tests are
        # included only on Unix (apptrash/mod.rs).
        windows:*/apptrash/*) ;;
        *) unexpected+=("$name ($defined_in)") ;;
    esac
done
if [[ "${#unexpected[@]}" -ne 0 ]]; then
    printf 'not compiled on %s: %s\n' "$platform" "${unexpected[@]}" >&2
    stage_failed "milestone tests defined in the source did not run"
fi
echo "android background task suite: ${#compiled_tests[@]} milestone tests compiled (${passed_line:-no passing summary})"

# ---------------------------------------------------------------------------
# Directly affected integrations: every test of every module this batch
# changed (Share client and host, daemon, mobile facade, storage analysis and
# duplicates, app trash, local access), so behavior the batch did not mean to
# change (desktop Share sessions, daemon stop and handoff, sync scheduling,
# analysis results outside protected folders) is checked as it is. One thread:
# several of these tests set process-wide state (the app trash volumes, the
# host state, the Share power profile). The opt-in switches match
# android/test-android-task.sh G2.
affected_modules=(
    share:: daemon:: mobile:: analytics:: apptrash:: local_access:: syncjobs:: cli::
)
skip_arguments=(--skip windows_analysis_task_tests)
modules_log="$suite_tmp/modules.log"
echo "android background task suite: tests of the affected modules"
if ! (
    cd "$repo_root/native"
    SMART_EXPLORER_COPY_PASTE_TASK=1 SMART_EXPLORER_GUI_TASK=1 run_task timeout --kill-after=60s 3600 \
        cargo test --locked --lib -- --test-threads=1 "${skip_arguments[@]}" "${affected_modules[@]}"
) 2>&1 | tee "$modules_log" ||
    ! grep -Eq '^test result: ok\. [0-9]+ passed; 0 failed' "$modules_log"; then
    report_hung_test "$modules_log"
    stage_failed "tests of the affected modules"
fi

if [[ "$platform" != linux ]]; then
    finish_suite "task-level suite passed on $platform"
    exit 0
fi

# ---------------------------------------------------------------------------
# The Share server: its complete test set (idle keepalive over both
# transports, presence bundling at the default interval, the relay ping
# schedule, and every older protocol test), plus its own milestone list.
server_log="$suite_tmp/share-server.log"
echo "android background task suite: Share server tests"
if ! (
    cd "$repo_root/share-server"
    CARGO_TARGET_DIR="$repo_root/share-server/target" run_task timeout --kill-after=60s 2400 \
        cargo test --locked -- --test-threads=1
) 2>&1 | tee "$server_log" ||
    ! grep -Eq '^test result: ok\. [0-9]+ passed; 0 failed' "$server_log"; then
    report_hung_test "$server_log"
    stage_failed "Share server tests"
fi
mapfile -t server_source_tests < <(
    grep -rhoE 'fn android_background_task_[A-Za-z0-9_]+' "$repo_root/share-server/src" |
        sed 's/^fn //' | sort -u
)
mapfile -t server_compiled_tests < <(
    { grep -oE '^test [^ ]+ \.\.\. ok' "$server_log" || true; } | awk '{print $2}' | sed 's/.*:://' | sort -u
)
mapfile -t server_missing < <(comm -23 <(printf '%s\n' "${server_source_tests[@]}") \
    <(printf '%s\n' "${server_compiled_tests[@]}"))
if [[ "${#server_source_tests[@]}" -eq 0 ]]; then
    stage_failed "the Share server has no android_background_task_ milestone tests"
elif [[ -n "${server_missing[*]}" ]]; then
    printf 'Share server milestone did not pass: %s\n' "${server_missing[@]}" >&2
    stage_failed "Share server milestone tests did not all pass"
fi
echo "android background task suite: ${#server_source_tests[@]} Share server milestone tests passed"

# ---------------------------------------------------------------------------
# Share end to end with the real CLI and Share server: a Room between two
# headless desktop clients through the changed server (relay ping schedule,
# keepalive capability negotiation), every file transaction and concurrent
# downloads in both directions.
e2e_log="$suite_tmp/share-e2e.log"
echo "android background task suite: Share Room end to end"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/native/target}"
e2e_built=true
(
    cd "$repo_root/native"
    run_task cargo build --locked --bin se
) || e2e_built=false
(
    cd "$repo_root/share-server"
    CARGO_TARGET_DIR="$repo_root/share-server/target" run_task cargo build --locked --bin se-share-server
) || e2e_built=false
if [[ "$e2e_built" != true ]]; then
    stage_failed "Share Room end to end: se or se-share-server did not build"
else
    e2e_tmp="$suite_tmp/share-e2e"
    mkdir -p "$e2e_tmp"
    TMPDIR="$e2e_tmp" \
    SMART_EXPLORER_SE_BINARY="$CARGO_TARGET_DIR/debug/se" \
    SMART_EXPLORER_SHARE_SERVER_BINARY="$repo_root/share-server/target/debug/se-share-server" \
        bash "$repo_root/native/test-share-room-e2e.sh" 2>&1 | tee "$e2e_log" || true
    grep -Fq 'Room lifecycle passed:' "$e2e_log" || stage_failed "Share Room end to end"
fi

finish_suite "task-level suite passed with batch gates, milestones, affected modules, Share server and Share end to end"
