#!/usr/bin/env bash
set -euo pipefail

export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
windows_target="x86_64-pc-windows-gnu"

command -v cargo >/dev/null 2>&1 || {
    echo "cargo is required" >&2
    exit 1
}

assert_contains() {
    local file=$1
    local needle=$2
    grep -Fq -- "$needle" "$file" || {
        echo "missing invariant in $file: $needle" >&2
        exit 1
    }
}

assert_absent() {
    local file=$1
    local needle=$2
    if grep -Fq -- "$needle" "$file"; then
        echo "forbidden invariant in $file: $needle" >&2
        exit 1
    fi
}

assert_before() {
    local file=$1
    local first=$2
    local second=$3
    local first_line second_line
    first_line="$(grep -nF -- "$first" "$file" | head -1 | cut -d: -f1)"
    second_line="$(grep -nF -- "$second" "$file" | head -1 | cut -d: -f1)"
    if [ -z "$first_line" ] || [ -z "$second_line" ] || [ "$first_line" -ge "$second_line" ]; then
        echo "expected '$first' before '$second' in $file" >&2
        exit 1
    fi
}

echo "remote-drive task suite: release memory boundary"
for release_leaf in \
    native/publish-feed.sh \
    native/publish-linux-feed-wsl.sh \
    native/build-agent-bundles.sh; do
    grep -Fxq 'export CARGO_BUILD_JOBS=1' "$repo_root/$release_leaf"
    grep -Fxq 'export CARGO_PROFILE_RELEASE_LTO=thin' "$repo_root/$release_leaf"
    grep -Fxq 'export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8' "$repo_root/$release_leaf"
done
grep -Fq 'lto = "thin"' "$repo_root/native/Cargo.toml"
grep -Fq 'codegen-units = 8' "$repo_root/native/Cargo.toml"
grep -Fq '$env:CARGO_BUILD_JOBS = "1"' "$repo_root/native/publish-release-local.ps1"
grep -Fq '$env:CARGO_PROFILE_RELEASE_LTO = "thin"' "$repo_root/native/publish-update.ps1"
grep -Fq '$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "8"' "$repo_root/native/publish-update.ps1"
grep -Fq '$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "8"' "$repo_root/native/publish-release-local.ps1"
test "$(grep -Fc 'run-release-memory-bounded.sh' "$repo_root/native/publish-release-local.ps1")" -eq 2
grep -Fxq '  -p MemoryHigh=3G' "$repo_root/native/run-release-memory-bounded.sh"
grep -Fxq '  -p MemoryMax=4G' "$repo_root/native/run-release-memory-bounded.sh"
grep -Fxq '  -p MemorySwapMax=1G' "$repo_root/native/run-release-memory-bounded.sh"
memory_settings="$("$repo_root/native/run-release-memory-bounded.sh" \
    bash -c 'printf "%s:%s:%s:%s:%s" "$CARGO_BUILD_JOBS" "$CARGO_PROFILE_RELEASE_LTO" "$CARGO_PROFILE_RELEASE_CODEGEN_UNITS" "$CARGO_PROFILE_TEST_DEBUG" "$CARGO_PROFILE_DEV_DEBUG"')"
case "$memory_settings" in
    *"1:thin:8:0:0") ;;
    *)
        echo "unexpected release memory settings: $memory_settings" >&2
        exit 1
        ;;
esac

echo "remote-drive task suite: source and installer invariants"
sftp_session="$repo_root/native/src/sftp/core/session.rs"
sftp_backend="$repo_root/native/src/sftp/core/backend.rs"
agent_deploy="$repo_root/native/src/agent/core/deploy.rs"
connector="$repo_root/native/src/connect/os/shared/connector.rs"
mount_manager="$repo_root/native/src/daemon/os/shared/mount_manager.rs"
ipc_host="$repo_root/native/src/daemon/os/shared/ipc_host.rs"
mount_source="$repo_root/native/src/daemon/os/shared/mount_source.rs"
mount_process="$repo_root/native/src/daemon/os/windows/mount_process.rs"
mount_process_environment="$repo_root/native/src/daemon/os/windows/mount_process_environment.rs"
mount_host_process="$repo_root/native/src/daemon/os/shared/mount_host_process.rs"
rooted_backend="$repo_root/native/src/daemon/os/shared/rooted_backend.rs"
rooted_backend_gate="$repo_root/native/src/daemon/os/shared/rooted_backend_gate.rs"
remote_open="$repo_root/native/src/app/os/shared/remote_open.rs"
mount_types="$repo_root/native/src/mount/core/types.rs"
metadata_policy="$repo_root/native/src/mount/core/metadata_policy.rs"
metadata_cache="$repo_root/native/src/mount/core/metadata_cache.rs"
metadata_loading="$repo_root/native/src/mount/core/metadata_loading.rs"
mount_metadata="$repo_root/native/src/mount/core/metadata.rs"
mount_host="$repo_root/native/src/mount/os/windows/host.rs"
metadata_refresh="$repo_root/native/src/mount/os/windows/metadata_refresh.rs"
metadata_callbacks="$repo_root/native/src/mount/os/windows/callbacks_metadata.rs"
open_callbacks="$repo_root/native/src/mount/os/windows/callbacks_open.rs"
io_callbacks="$repo_root/native/src/mount/os/windows/callbacks_io.rs"
mutation_callbacks="$repo_root/native/src/mount/os/windows/callbacks_mutation.rs"
mount_file_io="$repo_root/native/src/mount/core/file_io.rs"
mount_mutations="$repo_root/native/src/mount/core/mutations.rs"
mount_delete="$repo_root/native/src/mount/core/delete.rs"
mount_replace="$repo_root/native/src/mount/core/replace.rs"
mount_ui_draft="$repo_root/native/src/app/core/mount_ui_draft.rs"
drive_cli="$repo_root/native/src/cli/drive.rs"
peer_backend="$repo_root/native/src/share/core/backend.rs"
peer_lease="$repo_root/native/src/share/core/mount_lease.rs"
peer_lease_release="$repo_root/native/src/share/core/peer_lease_release.rs"
peer_lease_cleanup="$repo_root/native/src/share/core/mount_lease_cleanup.rs"
peer_endpoint_source="$repo_root/native/src/share/core/peer_endpoint_source.rs"
peer_node_sessions="$repo_root/native/src/share/core/node_sessions.rs"
peer_request="$repo_root/native/src/share/core/peer_request.rs"
peer_telemetry="$repo_root/native/src/share/core/peer_telemetry.rs"
peer_session="$repo_root/native/src/share/core/session.rs"
peer_mod="$repo_root/native/src/share/mod.rs"
peer_node="$repo_root/native/src/share/core/node.rs"
peer_node_accept="$repo_root/native/src/share/core/node_accept.rs"
peer_server="$repo_root/native/src/share/core/server.rs"
peer_service="$repo_root/native/src/share/core/service.rs"
peer_wire="$repo_root/native/src/share/core/wire.rs"
peer_mount_ui="$repo_root/native/src/app/core/mount_peer_roots.rs"
ipc_protocol="$repo_root/native/src/daemon/os/shared/ipc_protocol.rs"
agent_backend="$repo_root/native/src/agent/core/backend.rs"
agent_mux="$repo_root/native/src/agent/core/mux.rs"
agent_transport="$repo_root/native/src/agent/core/transport.rs"
sftp_connection="$repo_root/native/src/sftp/core/connection.rs"
sftp_reconnect_gate="$repo_root/native/src/sftp/core/reconnect_gate.rs"
runtime_download="$repo_root/native/src/mount/os/windows/runtime_install_download.rs"
runtime_process="$repo_root/native/src/mount/os/windows/runtime_install_process.rs"
installer="$repo_root/native/installer.nsi"

test "$(grep -Fc 'request_subsystem(true, "sftp")' "$sftp_session")" -eq 1
if rg -q 'RawSftpSession|posix-rename@openssh\.com' "$repo_root/native/src/sftp"; then
    echo "plain SFTP must use one standard subsystem without POSIX-rename coupling" >&2
    exit 1
fi
assert_contains "$sftp_session" 'Duration::from_millis(250)'
assert_contains "$sftp_backend" 'create: true'
assert_contains "$sftp_backend" 'replace: false'
assert_contains "$sftp_backend" 'namespace_replace: false'

assert_contains "$agent_deploy" 'agent_cache_path(&dir, &expected)'
assert_contains "$agent_deploy" 'inner.open_write_new(&tmp)'
assert_before "$agent_deploy" 'require_remote_sha256(&*inner, &tmp, &expected)?' 'sftp.exec_capture(&format!('
assert_contains "$agent_deploy" 'require_remote_sha256(&*inner, &remote, &expected)'
test "$(grep -Fc 'open_verified_agent(' "$agent_deploy")" -ge 3

assert_contains "$connector" 'AgentFallback::RequireConfined'
assert_contains "$connector" 'agent_fallback.permits_deploy_failure()'
assert_contains "$mount_source" 'open_saved_at_for_mount(connection, root.as_str(), config.root_security)'
assert_before "$mount_manager" 'start_cache::prepare(self, &key, &config.id)' 'super::mount_source::resolve(&config, host)'
assert_contains "$ipc_host" 'pub(super) mounts: super::mount_manager::MountManager'
assert_contains "$mount_manager" 'child: Option<super::mount_host_process::MountHostProcess>'

assert_contains "$mount_process" 'std::env::current_exe()?'
assert_absent "$mount_process" 'with_file_name("se.exe")'
assert_contains "$mount_process" 'GetSystemWindowsDirectoryW'
assert_contains "$mount_process" 'MAX_WINDOWS_DIRECTORY_UNITS'
assert_contains "$mount_process" 'MountHostProcess::capture_piped_stderr(command.spawn()?)'
assert_contains "$mount_process_environment" '.env_clear()'
assert_contains "$mount_process_environment" '.env("SystemRoot", system_windows_directory)'
assert_contains "$mount_process_environment" '.env("WINDIR", system_windows_directory)'
assert_before "$mount_process_environment" '.env_clear()' '.env("SystemRoot", system_windows_directory)'
assert_before "$mount_process_environment" '.env("WINDIR", system_windows_directory)' '.env(MOUNT_TOKEN_ENV, launch_token)'
assert_absent "$mount_process_environment" 'std::env::var_os('
assert_absent "$mount_process_environment" '.env("PATH"'
assert_absent "$mount_process_environment" '.env("TEMP"'
assert_contains "$mount_host_process" 'MOUNT_HOST_STDERR_LIMIT'
assert_contains "$repo_root/native/src/main.rs" 'run_host_if_requested(&arguments)'
assert_contains "$repo_root/native/src/bin/se.rs" 'run_host_if_requested(&arguments)'

assert_contains "$rooted_backend" 'inner.mount_path_capabilities(root.as_str())?'
assert_contains "$rooted_backend" 'operation: super::rooted_backend_gate::OperationGate'
assert_contains "$rooted_backend_gate" 'pub(super) struct OperationGate(RwLock<()>);'

# The mount host owns one transport-independent Backend for its whole process
# lifetime. Only root metadata is synchronous; deeper snapshots stay bounded
# and are filled/refreshed by the one background worker after Dokany is ready.
assert_contains "$mount_types" 'pub metadata: MountMetadataPolicy'
assert_contains "$mount_types" '#[serde(default)]'
assert_contains "$metadata_policy" 'DEFAULT_METADATA_PRELOAD_DEPTH: u8 = 2'
assert_contains "$metadata_policy" 'MAX_METADATA_PRELOAD_DEPTH: u8 = 4'
assert_contains "$drive_cli" 'MountMetadataPolicy::new(args.metadata_depth)'
assert_contains "$mount_ui_draft" '0..=crate::mount::MAX_METADATA_PRELOAD_DEPTH'
assert_contains "$mount_ui_draft" '.map(|config| config.with_metadata_policy(metadata))'
assert_contains "$mount_ui_draft" 'crate::mount::install_drive_runtime(None)'
assert_contains "$ipc_protocol" 'pub metadata: crate::mount::MountMetadataPolicy'
assert_contains "$metadata_loading" 'METADATA_PRELOAD_BATCH: usize = 8'
assert_contains "$metadata_loading" 'METADATA_REFRESH_BATCH: usize = 16'
assert_contains "$metadata_loading" 'self.load_directory_cached(&root, 0)'
assert_contains "$metadata_refresh" 'REFRESH_INTERVAL: Duration = Duration::from_secs(20)'
assert_contains "$metadata_refresh" '.preload_metadata_batch_while(|| is_stopped(&stop))'
assert_contains "$metadata_refresh" '.refresh_metadata_while(|| is_stopped(&stop))'
assert_contains "$metadata_cache" 'MAX_CACHED_DIRECTORIES: usize = 4_096'
assert_contains "$metadata_cache" 'MAX_CACHED_ENTRIES: usize = 50_000'
assert_contains "$metadata_cache" 'MAX_CACHED_BYTES: usize = 32 * 1024 * 1024'
assert_contains "$metadata_cache" 'MAX_CACHED_DIRECTORY_BYTES: usize = 4 * 1024 * 1024'
assert_contains "$metadata_cache" 'entries: Arc<[VfsMeta]>'
assert_contains "$metadata_cache" 'generation: u64'
assert_contains "$mount_metadata" 'io::Result<Arc<[VfsMeta]>>'
assert_contains "$metadata_loading" 'self.metadata_cache.generation()? != generation'
assert_contains "$metadata_callbacks" 'context.engine.list_dir_cached(path)?'
assert_contains "$metadata_callbacks" 'context.engine.stat_cached(path)?'
assert_absent "$metadata_callbacks" 'guard_long_with_context'
for safety_file in \
    "$open_callbacks" \
    "$io_callbacks" \
    "$mutation_callbacks" \
    "$mount_file_io" \
    "$mount_mutations" \
    "$mount_delete" \
    "$mount_replace"; do
    assert_absent "$safety_file" 'stat_cached('
    assert_absent "$safety_file" 'list_dir_cached('
done
assert_contains "$mount_file_io" 'self.invalidate_metadata(&state.remote_path, false);'
assert_contains "$mount_mutations" 'self.invalidate_metadata(source.backend(), true);'
assert_contains "$mount_delete" 'self.invalidate_metadata(&delete.original_path, true);'
assert_contains "$mount_replace" 'self.invalidate_metadata(destination_path.backend(), true);'
assert_before "$mount_host" 'engine.preload_metadata()' 'start_on_available_drive(&runtime'
assert_before "$mount_host" 'start_on_available_drive(&runtime' 'storage.start_metadata_refresh()'
assert_before "$mount_host" 'storage.request_metadata_refresh_stop();' 'filesystem.close();'
assert_before "$mount_host" 'filesystem.close();' 'storage.join_metadata_refresh();'
assert_before "$mount_host" 'drop(engine);' 'drop(cache_lease);'

assert_contains "$peer_request" 'Ctrl::Fs { req, lease }'
assert_contains "$peer_node" 'mount_leases: Arc::new(super::mount_lease::PeerMountLeases::default())'
assert_contains "$peer_node" 'pub(super) sessions: Mutex<HashMap<String, Connection>>'
assert_contains "$peer_node_sessions" 'if let Some(connection) = self.healthy_cached_session(key)?'
assert_contains "$peer_node_sessions" 'Some(current) if current.stable_id() == failed_generation => sessions.remove(key)'
assert_absent "$peer_node_sessions" 'failed.close('
assert_contains "$peer_session" 'format!("{kind}:{relation_id}:{}", endpoint.presence.node_id)'
assert_absent "$peer_session" 'format!("{kind}:{relation_id}:{}:{}", endpoint.presence.node_id'
assert_contains "$peer_endpoint_source" 'validate_identity(initial, &presence)?'
assert_contains "$peer_endpoint_source" 'Presence is routing evidence, not the lifetime of an'
assert_contains "$peer_backend" 'lease_request_id:'
assert_contains "$peer_lease" 'RELEASABLE_LEASE_PREFIX: &str = "se-mount-v2."'
assert_contains "$peer_lease" 'MAX_MOUNT_LEASES_PER_PRINCIPAL: usize = 4'
assert_contains "$peer_lease" 'MAX_MOUNT_LEASES_TOTAL: usize = 16'
assert_contains "$peer_lease_release" 'BEST_EFFORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5)'
assert_contains "$peer_lease_release" 'MAX_RELEASE_WORKERS: usize = 4'
assert_contains "$peer_lease_cleanup" 'super::blocking::run("Share dispose legacy mount leases"'
assert_contains "$peer_request" 'IDEMPOTENT_CONTROL_BUDGET: Duration = Duration::from_secs(40)'
assert_contains "$peer_request" 'FsRequest::ListDir { .. } | FsRequest::Stat { .. }'
assert_contains "$peer_request" 'FsRequest::ReleaseLease => matches!(response, FsResponse::Ok)'
assert_contains "$peer_telemetry" 'let _ = events.try_send(event);'
assert_contains "$peer_server" 'node.filesystem_authorization_epoch()'
assert_contains "$peer_lease" 'self.authorization_epoch != authorization_epoch'
assert_before "$peer_service" 'self.iroh.stop_sharing()' 'let send_result = self.cmds.send'
assert_contains "$peer_mod" '#[path = "core/node_accept.rs"]'
assert_contains "$peer_node" 'if !self.sharing_active.swap(false, Ordering::AcqRel)'
assert_contains "$peer_node" 'self.require_sharing_active()?;'
assert_before "$peer_node_accept" 'if node.require_sharing_active().is_err()' 'incoming.refuse();'
assert_contains "$peer_wire" 'MOUNT_PATH_CAPABILITY_CONTRACT_VERSION'
assert_contains "$peer_mount_ui" 'crate::daemon::probe_share_mount_capabilities'
assert_contains "$ipc_protocol" 'ProbeShareMount {'
assert_contains "$ipc_protocol" 'MountPathCapabilities {'

# Metadata requests have bounded recovery and may move to a replacement
# transport generation. Mutations already in flight drain on the old one and
# are never replayed implicitly.
assert_contains "$agent_backend" 'METADATA_REQUEST_TIMEOUT: Duration = Duration::from_secs(20)'
assert_contains "$agent_transport" 'pub(super) fn safe_call_timeout('
assert_contains "$agent_transport" 'self.reconnect.is_some()'
assert_contains "$agent_transport" 'mux.retire();'
assert_contains "$agent_transport" 'pub(super) fn mutation_call('
assert_contains "$agent_mux" 'retired: AtomicBool'
assert_contains "$agent_mux" 'if pending.is_empty() && self.retired.load(Ordering::Acquire)'
assert_contains "$sftp_connection" 'SFTP_METADATA_DEADLINE: Duration = Duration::from_secs(20)'
assert_contains "$sftp_connection" 'let deadline = AbsoluteDeadline::after(SFTP_METADATA_DEADLINE);'
assert_contains "$sftp_connection" 'tokio::time::timeout_at(expires, future)'
assert_contains "$sftp_reconnect_gate" 'pub(super) struct ReconnectGate<T>'
assert_contains "$sftp_reconnect_gate" 'pub(super) struct AbsoluteDeadline'

assert_contains "$remote_open" 'Publish the durable recovery manifest before handing the file to'
assert_before "$remote_open" 'if let Err(error) = sync_recovery_manifest(&self.remote_edits)' 'let process = self.launch_for_edit(&p, mode)'
assert_contains "$repo_root/native/src/app/core/init.rs" 'notice: recovery_notice'
assert_absent "$repo_root/native/src/app/core/init.rs" 'error_msg: recovery_notice'
assert_contains "$repo_root/native/src/app/core/table.rs" 'show_horizontal_file_table(ui'

assert_contains "$repo_root/native/dokany-runtime.nsh" '!define DOKANY_VERSION "2.3.1.1000"'
assert_contains "$repo_root/native/dokany-runtime.nsh" '!define DOKANY_API_VERSION "231"'
assert_contains "$repo_root/native/dokany-runtime.nsh" '!define DOKANY_DRIVER_PROTOCOL_VERSION "400"'
assert_contains "$repo_root/native/dokany-runtime.nsh" '!define DOKANY_MSI_SIZE "9269248"'
assert_contains "$repo_root/native/dokany-runtime.nsh" '!define DOKANY_MSI_SHA256 "69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465"'
assert_contains "$repo_root/native/dokany-runtime.nsh" '!define DOKANY_MSI_URL "https://github.com/dokan-dev/dokany/releases/download/v2.3.1.1000/Dokan_x64.msi"'
assert_contains "$runtime_download" 'manifest_value("DOKANY_DRIVER_PROTOCOL_VERSION")?'
assert_contains "$runtime_download" '.take(pinned.size.saturating_add(1))'
assert_contains "$runtime_download" 'https://release-assets.githubusercontent.com/'
assert_contains "$runtime_download" '.redirects(2)'
assert_contains "$runtime_process" '.share_mode(FILE_SHARE_READ)'
assert_absent "$runtime_process" 'FILE_SHARE_WRITE'
assert_absent "$runtime_process" 'FILE_SHARE_DELETE'
assert_contains "$runtime_process" 'FILE_FLAG_OPEN_REPARSE_POINT'
assert_contains "$runtime_process" 'FILE_FLAG_BACKUP_SEMANTICS'
assert_contains "$runtime_process" 'let parent_chain = lock_parent_chain(path)?'
assert_contains "$runtime_process" 'WinVerifyTrust('
assert_contains "$runtime_process" '.join("msiexec.exe")'
assert_contains "$runtime_process" 'OsStr::new("runas")'
assert_contains "$runtime_process" '/passive /norestart ADDLOCAL=DokanDriverFeature INSTALLDEVFILES=0'

assert_contains "$installer" 'Section "Dokany ${DOKANY_VERSION} / DLL-API ${DOKANY_API_VERSION} / Treiberprotokoll ${DOKANY_DRIVER_PROTOCOL_VERSION} für Remote-Laufwerke (UAC erforderlich)" SEC_DOKANY'
test "$(grep -Fc 'SectionIn RO' "$installer")" -eq 1
assert_contains "$installer" 'Call CheckDokanyRuntime'
assert_contains "$installer" 'drive install-runtime --msi'
assert_contains "$installer" '/INSTALLDOKANY=1'
assert_contains "$installer" '!insertmacro UnselectSection ${SEC_DOKANY}'
uninstall_block="$(sed -n '/^Section "Uninstall"/,$p' "$installer")"
if printf '%s\n' "$uninstall_block" | rg -q 'msiexec|drive install-runtime|Dokan_x64\.msi'; then
    echo "Smart Explorer uninstall must not remove or reinstall Dokany" >&2
    exit 1
fi
for build_path in \
    "$repo_root/native/publish-feed.sh" \
    "$repo_root/native/publish-update.ps1"; do
    assert_contains "$build_path" 'DOKANY_MSI_SRC'
done
assert_contains "$repo_root/native/publish-release-local.ps1" 'fetch-dokany-runtime.ps1'

for shell_script in \
    "$repo_root/native/fetch-dokany-runtime.sh" \
    "$repo_root/native/publish-feed.sh" \
    "$repo_root/native/publish-linux-feed-wsl.sh" \
    "$repo_root/native/build-agent-bundles.sh" \
    "$repo_root/native/run-release-memory-bounded.sh"; do
    bash -n "$shell_script"
done
if command -v pwsh >/dev/null 2>&1; then
    for powershell_script in \
        "$repo_root/native/fetch-dokany-runtime.ps1" \
        "$repo_root/native/publish-update.ps1" \
        "$repo_root/native/publish-release-local.ps1"; do
        SMART_EXPLORER_PARSE_FILE="$powershell_script" \
            pwsh -NoProfile -NonInteractive -Command \
                '$ErrorActionPreference="Stop"; [void][scriptblock]::Create([IO.File]::ReadAllText($env:SMART_EXPLORER_PARSE_FILE))'
    done
fi

if command -v rustup >/dev/null 2>&1; then
    rustup target list --installed | grep -Fxq "$windows_target" || {
        echo "missing Rust target: $windows_target" >&2
        exit 1
    }
fi

echo "remote-drive task suite: native behavior"
(
    cd "$repo_root/native"
    "$repo_root/native/run-release-memory-bounded.sh" \
        cargo test --locked --lib remote_drive_task_ -- --test-threads=1
)

echo "remote-drive task suite: confined agent process"
(
    cd "$repo_root/se-agent"
    "$repo_root/native/run-release-memory-bounded.sh" \
        cargo test --locked --test remote_drive_task remote_drive_task_agent_ -- --test-threads=1
)

echo "remote-drive task suite: Windows host boundary"
(
    cd "$repo_root/native"
    "$repo_root/native/run-release-memory-bounded.sh" \
        cargo check --locked --bin smart_explorer --bin se --target "$windows_target"
)
