#!/usr/bin/env bash
# Single task-level suite for the streaming transfer engine batch
# (docs/superpowers/plans/2026-09-28-transfer-engine/). Run this one checked-in
# entrypoint on the remote runner with an outer timeout of at least 30 minutes.
#
#   --check   narrow compile check while the batch is being built: the batch
#             gates only (rustfmt of every changed file, clippy for the host and
#             the Windows target over all targets; diagnostics on changed lines
#             are reported). No tests run.
#   (none)    the complete suite: gating batch gates, every milestone test
#             (`transfer_engine_task_`), the copy/paste safety tests, the tests
#             of every module the batch touched and, on Linux, transfers
#             against SFTP/FTP containers (plain and over a shaped link) and
#             the Share Room end-to-end run with real binaries.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-transfer-engine-task.sh [--check] [--bounded|--direct]" >&2
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
    echo "transfer engine task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-transfer-engine-task.XXXXXX")"
fi
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -f "$suite_tmp/batch-ranges.txt" "$suite_tmp"/clippy-*.log
        rmdir "$suite_tmp" 2>/dev/null || true
    else
        echo "transfer engine task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in awk cargo comm git grep mktemp rustfmt sed sort tee tr uname; do
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

echo "transfer engine task suite: batch gates ($platform, $suite_mode)"
# The crate as a whole carries older formatting and dead-code drift outside
# this batch (docs/TODO.md, H1), so the gates cover exactly what this batch
# touched: every changed file is format-checked on its own through rustfmt's
# stdin mode, and clippy runs over all targets of the crate, but only
# diagnostics on lines this batch changed fail. A compile error anywhere still
# fails. The batch base is the last commit before the batch.
batch_base=6535f767c08314e051e99b1c53ec634c6b8a9525
if ! git -C "$repo_root" cat-file -e "${batch_base}^{commit}" 2>/dev/null; then
    git -C "$repo_root" fetch --quiet --depth=1 origin "$batch_base"
fi
mapfile -t batch_files < <(
    git -C "$repo_root" diff --name-only --diff-filter=AM "$batch_base" HEAD -- \
        'native/src/*.rs' 'native/tests/*.rs'
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
    echo "transfer engine task suite: FAILED: $1" >&2
    if [[ "$suite_mode" == check ]]; then
        exit 1
    fi
}
finish_suite() {
    if [[ "${#failed_stages[@]}" -ne 0 ]]; then
        printf 'transfer engine task suite: failed stage: %s\n' "${failed_stages[@]}" >&2
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
    echo "transfer engine task suite: ${#batch_files[@]} batch source files are rustfmt-clean"
fi

batch_ranges="$suite_tmp/batch-ranges.txt"
: > "$batch_ranges"
for batch_file in "${batch_files[@]}"; do
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
    echo "transfer engine task suite: clippy ($clippy_target) is clean on the lines this batch changed"
done

if [[ "$suite_mode" == check ]]; then
    suite_succeeded=true
    echo "transfer engine task suite: batch gates passed (check)"
    exit 0
fi

# ---------------------------------------------------------------------------
# Milestone tests: every `transfer_engine_task_` test of the library. The
# compiled list is the truth of what runs; the source is compared against it
# so a test module that silently stopped compiling (a lost `mod` line, a
# wrong cfg) fails the suite instead of shrinking it.
milestone_log="$suite_tmp/milestones.log"
echo "transfer engine task suite: milestone tests ($platform)"
if ! (
    cd "$repo_root/native"
    run_task cargo test --locked --lib transfer_engine_task_ -- --test-threads=1
) 2>&1 | tee "$milestone_log"; then
    stage_failed "milestone tests"
fi
passed_line="$(grep -E '^test result: ok\. [0-9]+ passed; 0 failed' "$milestone_log" | head -n 1 || true)"
mapfile -t source_tests < <(
    grep -rhoE 'fn transfer_engine_task_[A-Za-z0-9_]+' "$repo_root/native/src" |
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
        windows) grep -Eq 'cfg\((all\()?(unix|target_family = "unix"|target_os = "(linux|android)"|not\(windows\))' <<<"$1" ;;
        linux) grep -Eq 'cfg\((all\()?(windows|target_family = "windows"|target_os = "windows"|not\(unix\))' <<<"$1" ;;
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
        *) unexpected+=("$name ($defined_in)") ;;
    esac
done
if [[ "${#unexpected[@]}" -ne 0 ]]; then
    printf 'not compiled on %s: %s\n' "$platform" "${unexpected[@]}" >&2
    stage_failed "milestone tests defined in the source did not run"
fi
echo "transfer engine task suite: ${#compiled_tests[@]} milestone tests compiled (${passed_line:-no passing summary})"

# Milestones that need the isolated, sequential task runner (the Windows
# clipboard round trip through OLE) are ignored by default; this throwaway
# runner is one, so they run here with their switch.
mapfile -t runner_milestones < <(
    { grep -oE '^test [^ ]+ \.\.\. ignored' "$milestone_log" || true; } | awk '{print $2}' | sort -u
)
if [[ "${#runner_milestones[@]}" -ne 0 ]]; then
    runner_log="$suite_tmp/milestones-runner.log"
    echo "transfer engine task suite: ${#runner_milestones[@]} runner-only milestone tests"
    if ! (
        cd "$repo_root/native"
        SMART_EXPLORER_COPY_PASTE_TASK=1 run_task cargo test --locked --lib -- --ignored --exact \
            --test-threads=1 "${runner_milestones[@]}"
    ) 2>&1 | tee "$runner_log" ||
        ! grep -Eq "^test result: ok\. ${#runner_milestones[@]} passed; 0 failed" "$runner_log"; then
        stage_failed "runner-only milestone tests: ${runner_milestones[*]}"
    fi
fi

# ---------------------------------------------------------------------------
# The copy/paste safety tests (foreign data survives, no unsafe fallback,
# changed sources stay unpublished, acknowledged commits count) through the
# engine; they are ignored by default and need their fixture switch.
copy_paste_log="$suite_tmp/copy-paste.log"
echo "transfer engine task suite: copy/paste safety tests"
if ! (
    cd "$repo_root/native"
    SMART_EXPLORER_COPY_PASTE_TASK=1 run_task cargo test --locked --lib copy_paste_task_transfer_ \
        -- --ignored --test-threads=1
) 2>&1 | tee "$copy_paste_log" ||
    ! grep -Eq '^test result: ok\. 9 passed; 0 failed' "$copy_paste_log"; then
    stage_failed "the nine copy/paste safety tests"
fi

# ---------------------------------------------------------------------------
# Directly affected integrations: every test of every module this batch
# changed, so behavior the batch did not mean to change (sync, mounts,
# remote paths, stored locations, the folder picker, CLI transfers) is
# checked as it is. One thread: several of these tests set process-wide
# state (the app trash volumes, environment switches). The opt-in switches
# of the suites these tests come from are set, as android/test-android-task.sh
# G2 sets them: the App task constructors and the loopback Share peer fixture
# refuse to run without them.
affected_modules=(
    transfer:: copy:: vfs:: net:: sync:: bisync:: syncjobs:: connect:: gdrive:: ftp:: webdav::
    sftp:: smb:: zipfs:: agent:: agent_proto:: daemon:: share:: app:: cli:: mobile::
    virtual_clipboard:: dragout::
)
# The daemon's Windows analysis bridge needs the Windows remote suite's own
# switch and isolated profile (native/test-windows-remote-task.ps1).
skip_arguments=(--skip windows_analysis_task_tests)
if [[ "$platform" == windows ]]; then
    # Fails on Windows since before this batch (docs/TODO.md, H1).
    skip_arguments+=(--skip vfs::tests::copy_file_default_impl_streams)
fi
modules_log="$suite_tmp/modules.log"
echo "transfer engine task suite: tests of the affected modules"
if ! (
    cd "$repo_root/native"
    SMART_EXPLORER_COPY_PASTE_TASK=1 SMART_EXPLORER_GUI_TASK=1 run_task cargo test --locked --lib -- \
        --test-threads=1 "${skip_arguments[@]}" "${affected_modules[@]}"
) 2>&1 | tee "$modules_log" ||
    ! grep -Eq '^test result: ok\. [0-9]+ passed; 0 failed' "$modules_log"; then
    stage_failed "tests of the affected modules"
fi

if [[ "$platform" != linux ]]; then
    finish_suite "task-level suite passed on $platform"
    exit 0
fi

# ---------------------------------------------------------------------------
# Real servers: SFTP (OpenSSH) and FTP (vsftpd) containers on loopback.
for command_name in docker jq timeout cmp head sudo tc comm; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "$command_name is required for the server checks" >&2
        exit 1
    }
done
sftp_port=2222
sftp_user=setester
sftp_pass=se-task-sftp-pass
ftp_user=seftp
ftp_pass=se-task-ftp-pass
netem_active=false
stop_servers() {
    if [[ "$netem_active" == true ]]; then
        sudo -n tc qdisc del dev lo root 2>/dev/null || true
        netem_active=false
    fi
    docker rm -f se-engine-sftp se-engine-ftp >/dev/null 2>&1 || true
}
trap 'stop_servers; cleanup' EXIT
wait_banner() {
    local name=$1 port=$2 prefix=$3 deadline=$((SECONDS + 180)) banner=""
    while ((SECONDS < deadline)); do
        banner="$(timeout 5 bash -c "exec 3<>/dev/tcp/127.0.0.1/$port && head -c 8 <&3" 2>/dev/null || true)"
        if [[ "$banner" == "$prefix"* ]]; then
            echo "test server $name ready on port $port"
            return 0
        fi
        sleep 1
    done
    echo "test server $name did not answer on port $port" >&2
    docker logs "se-engine-$name" >&2 || true
    return 1
}
stop_servers
servers_ready=true
docker run -d --name se-engine-sftp -p "$sftp_port:22" atmoz/sftp:latest \
    "$sftp_user:$sftp_pass:::upload" >/dev/null || servers_ready=false
# vsftpd on the host network: its data connections need no published port
# range (a PASV that finds no free port in its few random tries ends the
# session), and the shaped loopback carries them directly. No client
# limits: the engine's own flow finds the server's capacity.
docker run -d --name se-engine-ftp --network host \
    -e USERS="$ftp_user|$ftp_pass" delfer/alpine-ftp-server:latest \
    vsftpd /etc/vsftpd/vsftpd.conf -obackground=NO -opasv_min_port=21000 \
    -opasv_max_port=21999 -opasv_address=127.0.0.1 -omax_clients=0 -omax_per_ip=0 \
    >/dev/null || servers_ready=false
if [[ "$servers_ready" == true ]]; then
    wait_banner sftp "$sftp_port" "SSH-" || servers_ready=false
    wait_banner ftp 21 "220" || servers_ready=false
fi
if [[ "$servers_ready" != true ]]; then
    stage_failed "the SFTP/FTP test servers did not start (server and throughput checks not run)"
elif ! (
    cd "$repo_root/native"
    run_task cargo test --locked --test transfer_containers --test transfer_throughput --no-run
); then
    stage_failed "the server and throughput checks did not build"
else
    export SE_TASK_SFTP="127.0.0.1:$sftp_port:$sftp_user:$sftp_pass:/upload"
    export SE_TASK_FTP_URL="ftp://$ftp_user:$ftp_pass@127.0.0.1:21/ftp/$ftp_user"
    containers_log="$suite_tmp/containers.log"
    echo "transfer engine task suite: whole trees against the servers"
    if ! (
        cd "$repo_root/native"
        run_task cargo test --locked --test transfer_containers -- --ignored --test-threads=1
    ) 2>&1 | tee "$containers_log" ||
        ! grep -Eq '^test result: ok\. 6 passed; 0 failed' "$containers_log"; then
        stage_failed "transfers against the servers"
    fi

    # Shaped loopback: 25 ms each way (50 ms round trip) at 100 Mbit/s, a
    # common remote link, so concurrency and pipelining have to earn the
    # throughput.
    throughput_log="$suite_tmp/throughput.log"
    echo "transfer engine task suite: throughput over a shaped link (50 ms, 100 Mbit/s)"
    if sudo -n tc qdisc add dev lo root netem delay 25ms rate 100mbit; then
        netem_active=true
        (
            cd "$repo_root/native"
            SE_TASK_NETEM_RTT_MS=50 SE_TASK_NETEM_RATE_MBIT=100 run_task cargo test --locked \
                --test transfer_throughput -- --ignored --test-threads=1 --nocapture
        ) 2>&1 | tee "$throughput_log" || true
        sudo -n tc qdisc del dev lo root || true
        netem_active=false
        grep -E '^throughput ' "$throughput_log" || true
        grep -Eq '^test result: ok\. 5 passed; 0 failed' "$throughput_log" ||
            stage_failed "throughput over the shaped link missed its bounds"
    else
        stage_failed "the shaped link (netem on loopback) could not be set up"
    fi
fi
stop_servers

# ---------------------------------------------------------------------------
# Share end to end with the real CLI and Share server: a Room between two
# headless clients, every file transaction and concurrent downloads in both
# directions over the new transfer protocol and the service's credit flow.
e2e_log="$suite_tmp/share-e2e.log"
echo "transfer engine task suite: Share Room end to end"
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
    SMART_EXPLORER_SE_BINARY="$CARGO_TARGET_DIR/debug/se" \
    SMART_EXPLORER_SHARE_SERVER_BINARY="$repo_root/share-server/target/debug/se-share-server" \
        bash "$repo_root/native/test-share-room-e2e.sh" 2>&1 | tee "$e2e_log" || true
    grep -Fq 'Room lifecycle passed:' "$e2e_log" || stage_failed "Share Room end to end"
fi

finish_suite "task-level suite passed with milestones, safety tests, affected modules, servers, throughput and Share end to end"
