#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
se_bin="${SMART_EXPLORER_SE_BINARY:-$repo_root/native/target/debug/se}"
server_bin="${SMART_EXPLORER_SHARE_SERVER_BINARY:-$repo_root/share-server/target/debug/se-share-server}"

if [[ "${SMART_EXPLORER_BUILD_E2E_BINARIES:-0}" == 1 ]]; then
  (
    cd "$repo_root/native"
    cargo build --locked --bin se
  )
fi

command -v jq >/dev/null || {
  echo "share lifecycle E2E requires jq" >&2
  exit 1
}
command -v timeout >/dev/null || {
  echo "share lifecycle E2E requires GNU timeout" >&2
  exit 1
}
command -v pwsh >/dev/null || {
  echo "share lifecycle E2E requires PowerShell for release-script parsing" >&2
  exit 1
}
command -v python3 >/dev/null || {
  echo "share lifecycle E2E requires Python for workflow transaction validation" >&2
  exit 1
}
python3 -c 'import yaml' >/dev/null 2>&1 || {
  echo "share lifecycle E2E requires the Python yaml module for workflow validation" >&2
  exit 1
}
test -x "$se_bin" || {
  echo "se test binary is missing: $se_bin" >&2
  exit 1
}
test -x "$server_bin" || {
  echo "share-server test binary is missing: $server_bin" >&2
  exit 1
}

root="$(mktemp -d "${TMPDIR:-/tmp}/se-share-lifecycle.XXXXXX")"
client_a="$root/a"
client_b="$root/b"
client_c="$root/c"
client_d="$root/d"
server_log="$root/share-server.log"
server_pid=""

cleanup() {
  local status=$?
  stop_daemon "$client_a" || true
  stop_daemon "$client_b" || true
  stop_daemon "$client_c" || true
  stop_daemon "$client_d" || true
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ $status -ne 0 ]]; then
    echo "Share lifecycle E2E failed; diagnostics: $root" >&2
  fi
  if [[ $status -eq 0 && "${SMART_EXPLORER_KEEP_E2E_ROOT:-0}" != 1 ]]; then
    rm -rf "$root"
  fi
  return "$status"
}
trap cleanup EXIT

prepare_client() {
  local client="$1"
  mkdir -p "$client/home" "$client/data" "$client/config" "$client/runtime"
  chmod 700 "$client/home" "$client/data" "$client/config" "$client/runtime"
}

run_client() {
  local client="$1"
  shift
  local relay_override=()
  if [[ -s "$client/relay-url" ]]; then
    relay_override=("SE_SHARE_RELAY_URL=$(<"$client/relay-url")")
  fi
  timeout --foreground --signal=TERM --kill-after=5s 90s env \
    HOME="$client/home" \
    USERPROFILE="$client/home" \
    XDG_DATA_HOME="$client/data" \
    XDG_CONFIG_HOME="$client/config" \
    XDG_RUNTIME_DIR="$client/runtime" \
    APPDATA="$client/data" \
    LOCALAPPDATA="$client/data" \
    SE_SHARE_RELAY_ONLY=1 \
    "${relay_override[@]}" \
    "$se_bin" "$@"
}

# Use only for a background invocation. `exec` replaces Bash's asynchronous
# function subshell so `$!` is the actual `se` process, not a killable wrapper
# which could orphan the CLI under test.
run_client_background() {
  local client="$1"
  shift
  local relay_override=()
  if [[ -s "$client/relay-url" ]]; then
    relay_override=("SE_SHARE_RELAY_URL=$(<"$client/relay-url")")
  fi
  exec env \
    HOME="$client/home" \
    USERPROFILE="$client/home" \
    XDG_DATA_HOME="$client/data" \
    XDG_CONFIG_HOME="$client/config" \
    XDG_RUNTIME_DIR="$client/runtime" \
    APPDATA="$client/data" \
    LOCALAPPDATA="$client/data" \
    SE_SHARE_RELAY_ONLY=1 \
    "${relay_override[@]}" \
    "$se_bin" "$@"
}

daemon_pids() {
  local client="$1"
  local expected="XDG_DATA_HOME=$client/data"
  local env_file pid command
  for env_file in /proc/[0-9]*/environ; do
    [[ -r "$env_file" ]] || continue
    if tr '\0' '\n' 2>/dev/null <"$env_file" | grep -Fqx "$expected"; then
      pid="${env_file#/proc/}"
      pid="${pid%/environ}"
      command="$(tr '\0' ' ' 2>/dev/null <"/proc/$pid/cmdline" || true)"
      if [[ "$command" == *"--sync-daemon"* ]]; then
        printf '%s\n' "$pid"
      fi
    fi
  done
}

stop_daemon() {
  local client="$1"
  local pid
  while read -r pid; do
    [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
  done < <(daemon_pids "$client")
  local deadline=$((SECONDS + 10))
  while [[ $SECONDS -lt $deadline ]] && [[ -n "$(daemon_pids "$client")" ]]; do
    sleep 0.05
  done
  local remaining
  remaining="$(daemon_pids "$client")"
  if [[ -n "$remaining" ]]; then
    echo "Share daemon did not stop for $client: $remaining" >&2
    ps -o pid,ppid,stat,etime,cmd -p "${remaining//$'\n'/,}" >&2 || true
    return 1
  fi
}

wait_relay_route() {
  local client="$1"
  local expected_relay="$2"
  local signal_state="${3:-connected}"
  local deadline=$((SECONDS + 90))
  local value=""
  local connected_filter
  case "$signal_state" in
    connected) connected_filter='.worker.connected == true' ;;
    disconnected) connected_filter='.worker.connected == false' ;;
    any) connected_filter='true' ;;
    *)
      echo "invalid signaling state for relay wait: $signal_state" >&2
      return 2
      ;;
  esac
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share status --json 2>/dev/null)" \
      && jq -e --arg relay "$expected_relay" \
        ".worker.reachable == true and
         .worker.running == true and
         $connected_filter and
         ((.worker.relay_url | rtrimstr(\"/\")) == (\$relay | rtrimstr(\"/\"))) and
         (.worker.candidates | length) == 0" >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "relay-only route $expected_relay did not become ready for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

single_client_file() {
  local client="$1"
  local name="$2"
  local matches=()
  mapfile -t matches < <(find "$client" -type f -name "$name" -print)
  if [[ ${#matches[@]} -ne 1 ]]; then
    echo "expected one $name below $client, found ${#matches[@]}" >&2
    return 1
  fi
  printf '%s\n' "${matches[0]}"
}

rewrite_direct_presence() {
  local profile="$1"
  local contact_id="$2"
  local relay_url="$3"
  local expires_at="$4"
  local staged="$profile.route-stage"
  jq -e \
    --arg contact_id "$contact_id" \
    --arg relay_url "$relay_url" \
    --argjson expires_at "$expires_at" \
    'if ([.direct_contacts[] | select(.id == $contact_id and .presence != null)] | length) != 1
     then error("expected one persisted direct presence")
     else .schema_version = 6
       | (.direct_contacts[] | select(.id == $contact_id) | .presence.relay_url) = $relay_url
       | (.direct_contacts[] | select(.id == $contact_id) | .presence.candidates) = []
       | (.direct_contacts[] | select(.id == $contact_id) | .presence.expires_at) = $expires_at
     end' "$profile" >"$staged"
  mv "$staged" "$profile"
}

verify_release_transaction_scripts() {
  bash -n \
    "$repo_root/install-linux.sh" \
    "$repo_root/native/publish-feed.sh" \
    "$repo_root/native/test-share-lifecycle-e2e.sh"

  SMART_EXPLORER_TASK_SUITE_ROOT="$repo_root" \
    pwsh -NoProfile -NonInteractive -Command '
    $ErrorActionPreference = "Stop"
    $root = $env:SMART_EXPLORER_TASK_SUITE_ROOT
    if ([string]::IsNullOrWhiteSpace($root)) {
      throw "Task-suite repository root was not exported"
    }
    $paths = @(
      (Join-Path $root "native/publish-release-local.ps1"),
      (Join-Path $root "native/release-publication.ps1")
    )
    foreach ($path in $paths) {
      $tokens = $null
      $errors = $null
      [void][System.Management.Automation.Language.Parser]::ParseFile(
        $path,
        [ref]$tokens,
        [ref]$errors
      )
      if ($errors.Count -ne 0) {
        throw "PowerShell parser rejected ${path}: $($errors[0].Message)"
      }
    }
    . (Join-Path $root "native/release-publication.ps1")
    foreach ($name in @(
      "Assert-PublicationInstallerPayloads",
      "Assert-PublicationFallbackAdvance",
      "Assert-PublicationNoUntrackedBuildInputs",
      "Assert-ReleasePublicationCandidate",
      "Get-PublicationExpectedSourceCommit",
      "Get-ReleasePublicationGitHubToken",
      "Invoke-ReleasePublicationGitHubPost",
      "Invoke-ReleasePublicationCommit",
      "Invoke-ReleasePublicationMainPush",
      "Invoke-ReleasePublicationTagPush",
      "Test-PublicationPendingReleaseChanges",
      "Wait-ReleasePublicationWorkflow",
      "Wait-ReleasePublicationAssets",
      "Invoke-ReleasePublicationLinuxCliUpdate"
    )) {
      if (-not (Get-Command $name -CommandType Function -ErrorAction SilentlyContinue)) {
        throw "Release publication helper is missing $name"
      }
    }
    Assert-PublicationNoUntrackedBuildInputs -RepoRoot $root
    $currentVersion = Get-PublicationCargoVersion (Join-Path $root "native/Cargo.toml")
    $expectedSource = Get-PublicationExpectedSourceCommit -RepoRoot $root -Version $currentVersion
    $head = (Invoke-ReleasePublicationGit -RepoRoot $root -Arguments @("rev-parse", "HEAD")).StdOut.Trim().ToLowerInvariant()
    $subject = (Invoke-ReleasePublicationGit -RepoRoot $root -Arguments @("show", "-s", "--format=%s", "HEAD")).StdOut.Trim()
    $independentSource = if (
      $subject -eq "Release Smart Explorer v$currentVersion [release candidate]" -and
      -not (Test-PublicationPendingReleaseChanges -RepoRoot $root -Version $currentVersion)
    ) {
      (Invoke-ReleasePublicationGit -RepoRoot $root -Arguments @("rev-parse", "HEAD^")).StdOut.Trim().ToLowerInvariant()
    } else {
      $head
    }
    if ($expectedSource -ne $independentSource) {
      throw "Release build provenance does not bind the expected source commit"
    }
    $fixture = Join-Path ([IO.Path]::GetTempPath()) (
      "se-release-provenance-" + [Guid]::NewGuid().ToString("N")
    )
    try {
      $null = New-Item -ItemType Directory -Path (Join-Path $fixture "native") -Force
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("init", "--quiet")
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("config", "user.name", "Task Suite")
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("config", "user.email", "task-suite@example.invalid")
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("config", "commit.gpgsign", "false")
      Set-Content -LiteralPath (Join-Path $fixture "native/Cargo.toml") -Value "version = `"9.8.6`""
      Set-Content -LiteralPath (Join-Path $fixture "source.txt") -Value "source"
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("add", "--", "native/Cargo.toml", "source.txt")
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("commit", "-m", "source baseline", "--")
      $sourceParent = (Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("rev-parse", "HEAD")).StdOut.Trim().ToLowerInvariant()

      Set-Content -LiteralPath (Join-Path $fixture "native/Cargo.toml") -Value "version = `"9.8.7`""
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("add", "--", "native/Cargo.toml")
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @(
        "commit", "-m", "Release Smart Explorer v9.8.7 [release candidate]", "--"
      )
      $firstCandidate = (Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("rev-parse", "HEAD")).StdOut.Trim().ToLowerInvariant()
      $cleanExpected = Get-PublicationExpectedSourceCommit -RepoRoot $fixture -Version "9.8.7"
      if ($cleanExpected -ne $sourceParent) {
        throw "Clean release candidate provenance does not bind its sole parent"
      }

      Add-Content -LiteralPath (Join-Path $fixture "native/Cargo.toml") -Value "# replacement build"
      if (-not (Test-PublicationPendingReleaseChanges -RepoRoot $fixture -Version "9.8.7")) {
        throw "Replacement release changes were not detected"
      }
      $replacementExpected = Get-PublicationExpectedSourceCommit -RepoRoot $fixture -Version "9.8.7"
      if ($replacementExpected -ne $firstCandidate) {
        throw "Replacement build provenance does not bind the interrupted candidate HEAD"
      }
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("restore", "--worktree", "--", "native/Cargo.toml")

      Set-Content -LiteralPath (Join-Path $fixture "source.txt") -Value "unexpected candidate source change"
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @("add", "--", "source.txt")
      $null = Invoke-ReleasePublicationGit -RepoRoot $fixture -Arguments @(
        "commit", "-m", "Release Smart Explorer v9.8.8 [release candidate]", "--"
      )
      $rejected = $false
      try {
        $null = Get-PublicationExpectedSourceCommit -RepoRoot $fixture -Version "9.8.8"
      } catch {
        $rejected = $_.Exception.Message -like "*non-release source changes*"
      }
      if (-not $rejected) {
        throw "Local release candidate validation accepted a source-path commit"
      }
      $null = New-Item -ItemType Directory -Path (Join-Path $fixture ".cargo") -Force
      Set-Content -LiteralPath (Join-Path $fixture ".cargo/config.toml") -Value "[build]"
      $untrackedRejected = $false
      try {
        Assert-PublicationNoUntrackedBuildInputs -RepoRoot $fixture
      } catch {
        $untrackedRejected = $_.Exception.Message -like "*Untracked release build inputs*"
      }
      if (-not $untrackedRejected) {
        throw "Untracked root Cargo configuration was accepted as a release build input"
      }
    } finally {
      Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue
    }
    $assets = @(Get-PublicationReleaseAssetMap -RepoRoot $root -Version $currentVersion)
    if ($assets.Count -ne 18) {
      throw "Current release candidate does not map exactly 18 assets"
    }
    foreach ($asset in $assets) {
      Assert-PublicationNonEmptyFile $asset.LocalPath
    }
    Assert-PublicationInstallerPayloads `
      -RepoRoot $root `
      -Installer (Join-Path $root "release-native/Smart Explorer Setup $currentVersion.exe") `
      -Feed (Join-Path $root "release-native/update-feed")
    foreach ($payload in @(
      "smart_explorer.exe", "smart_explorer_updater.exe", "se.exe",
      "smart_explorer", "smart_explorer_updater", "se"
    )) {
      $null = Assert-PublicationHashSidecar `
        -Feed (Join-Path $root "release-native/update-feed") `
        -PayloadName $payload
    }
    $lockText = Get-Content -LiteralPath (Join-Path $root "native/Cargo.lock") -Raw
    $lockPattern = [regex]::new(
      "(?ms)(^\[\[package\]\]\r?\nname = `"smart_explorer`"\r?\nversion = `")([^`"]+)(`")"
    )
    $lockMatches = $lockPattern.Matches($lockText)
    if ($lockMatches.Count -ne 1 -or $lockMatches[0].Groups[2].Value -ne $currentVersion) {
      throw "Cargo.lock does not expose one controllable smart_explorer root version"
    }
    $simulatedVersion = "9.8.7"
    $simulatedLock = $lockPattern.Replace(
      $lockText,
      { param($match) "$($match.Groups[1].Value)$simulatedVersion$($match.Groups[3].Value)" },
      1
    )
    $simulatedMatches = $lockPattern.Matches($simulatedLock)
    if ($simulatedMatches.Count -ne 1 -or $simulatedMatches[0].Groups[2].Value -ne $simulatedVersion) {
      throw "Controlled Cargo.lock root-version update is not deterministic"
    }
    $wrapper = Get-Content -LiteralPath (Join-Path $root "native/publish-release-local.ps1") -Raw
    $preflight = $wrapper.IndexOf("`$preflightPlan = Assert-CommonReleasePreflight")
    $lock = $wrapper.IndexOf("`$completeReleaseLock = Enter-CompleteReleaseLock")
    $versionBump = $wrapper.IndexOf("Set-NativeVersion `$plan.Version", $lock)
    $firstCandidateCheck = $wrapper.IndexOf(
      "if (-not (Test-CompleteReleaseCandidateAvailable `$version))",
      $lock
    )
    $linuxBuild = $wrapper.IndexOf("Invoke-LinuxCompleteReleaseBuild `$completeReleaseLock", $firstCandidateCheck)
    $windowsBuild = $wrapper.IndexOf("Invoke-WindowsReleaseBuild", $firstCandidateCheck)
    $secondCandidateCheck = $wrapper.IndexOf(
      "if (-not (Test-CompleteReleaseCandidateAvailable `$version))",
      $firstCandidateCheck + 1
    )
    $commit = $wrapper.IndexOf("`$commit = Invoke-ReleasePublicationCommit", $secondCandidateCheck)
    $committedCandidateCheck = $wrapper.IndexOf(
      "`$null = Assert-ReleasePublicationCandidate -RepoRoot `$repoRoot -Version `$version",
      $commit
    )
    $mainPush = $wrapper.IndexOf("Invoke-ReleasePublicationMainPush", $committedCandidateCheck)
    $tag = $wrapper.IndexOf("Invoke-ReleasePublicationTagPush", $mainPush)
    $workflow = $wrapper.IndexOf("Wait-ReleasePublicationWorkflow", $tag)
    $assets = $wrapper.IndexOf("Wait-ReleasePublicationAssets", $workflow)
    $tagSha = $wrapper.IndexOf("`$publishedTagCommit = Get-RemoteTagCommit", $assets)
    $localCli = $wrapper.IndexOf("Invoke-ReleasePublicationLinuxCliUpdate", $tagSha)
    if ($preflight -lt 0 -or $lock -le $preflight -or
        $versionBump -le $lock -or $firstCandidateCheck -le $versionBump -or
        $linuxBuild -le $firstCandidateCheck -or
        $windowsBuild -le $firstCandidateCheck -or
        $secondCandidateCheck -le $linuxBuild -or $secondCandidateCheck -le $windowsBuild -or
        $commit -le $secondCandidateCheck -or
        $committedCandidateCheck -le $commit -or $mainPush -le $committedCandidateCheck -or
        $tag -le $mainPush -or $workflow -le $tag -or $assets -le $workflow -or
        $tagSha -le $assets -or $localCli -le $tagSha) {
      throw "Complete release transaction stages are missing or out of order"
    }
    if (-not $wrapper.Contains("-RetryFailedOnce:")) {
      throw "Tagged publication recovery does not reuse the existing workflow run"
    }
    if (-not $wrapper.Contains("`$buildSourceCommit") -or
        -not $wrapper.Contains("Assert-WindowsManifest `$stageFeed `$version `$buildSourceCommit")) {
      throw "Isolated Windows/WSL release staging is not bound to its captured source HEAD"
    }
    if (-not $wrapper.Contains(".Cargo.lock.complete-release-version.") -or
        -not $wrapper.Contains(".Cargo.toml.complete-release-version.") -or
        $wrapper.Contains(".complete-release-stage.version-")) {
      throw "Version files are not staged beside their atomic replacement targets"
    }
    $publicationHelpers = Get-Content -LiteralPath (Join-Path $root "native/release-publication.ps1") -Raw
    if (-not $publicationHelpers.Contains("`$run.conclusion -eq `"failure`"") -or
        -not $publicationHelpers.Contains("`"rerun-failed-jobs`"") -or
        -not $publicationHelpers.Contains("`"rerun`"")) {
      throw "Publication recovery does not distinguish failed-job and run-wide retries"
    }
    function New-TaskSuiteWorkflowRun([string]$Conclusion, [int]$Attempt) {
      return [pscustomobject]@{
        event = "push"
        head_sha = ("a" * 40)
        head_branch = "v9.8.7"
        path = ".github/workflows/build.yml"
        id = 987654
        status = "completed"
        conclusion = $Conclusion
        run_attempt = $Attempt
        html_url = "https://example.invalid/run/987654"
      }
    }
    function Invoke-ReleasePublicationGitHubGet {
      param([string]$RepositorySlug, [string]$ApiPath, [switch]$AllowNotFound)
      if ($script:taskSuiteRuns.Count -eq 0) {
        throw "Task-suite workflow response queue is empty"
      }
      return [pscustomobject]@{
        workflow_runs = @($script:taskSuiteRuns.Dequeue())
      }
    }
    function Invoke-ReleasePublicationGitHubPost {
      param([string]$RepositorySlug, [string]$ApiPath)
      [void]$script:taskSuitePosts.Add($ApiPath)
    }
    function Wait-ReleasePublicationDelay {
      param([datetimeoffset]$Deadline)
      return $true
    }
    function Reset-TaskSuiteWorkflowMocks {
      $script:taskSuiteRuns = [System.Collections.Generic.Queue[object]]::new()
      $script:taskSuitePosts = [System.Collections.Generic.List[string]]::new()
    }
    $workflowArgs = @{
      RepositorySlug = "owner/repository"
      Version = "9.8.7"
      CandidateSha = ("a" * 40)
      TriggerBranch = "v9.8.7"
      Deadline = [datetimeoffset]::UtcNow.AddMinutes(1)
    }

    Reset-TaskSuiteWorkflowMocks
    $script:taskSuiteRuns.Enqueue((New-TaskSuiteWorkflowRun "failure" 1))
    $script:taskSuiteRuns.Enqueue((New-TaskSuiteWorkflowRun "success" 2))
    $failedJobRetry = Wait-ReleasePublicationWorkflow @workflowArgs -RetryFailedOnce
    if ($failedJobRetry.RunAttempt -ne 2 -or $script:taskSuitePosts.Count -ne 1 -or
        $script:taskSuitePosts[0] -ne "/actions/runs/987654/rerun-failed-jobs") {
      throw "Failed publication jobs are not retried once on the same run"
    }

    Reset-TaskSuiteWorkflowMocks
    $script:taskSuiteRuns.Enqueue((New-TaskSuiteWorkflowRun "cancelled" 1))
    $script:taskSuiteRuns.Enqueue((New-TaskSuiteWorkflowRun "success" 2))
    $cancelledRetry = Wait-ReleasePublicationWorkflow @workflowArgs -RetryFailedOnce
    if ($cancelledRetry.RunAttempt -ne 2 -or $script:taskSuitePosts.Count -ne 1 -or
        $script:taskSuitePosts[0] -ne "/actions/runs/987654/rerun") {
      throw "Cancelled publication is not retried once on the same whole run"
    }

    Reset-TaskSuiteWorkflowMocks
    $script:taskSuiteRuns.Enqueue((New-TaskSuiteWorkflowRun "failure" 1))
    $retryWasRequired = $false
    try {
      $null = Wait-ReleasePublicationWorkflow @workflowArgs
    } catch {
      $retryWasRequired = $_.Exception.Message -like "*attempt 1 completed*failure*"
    }
    if (-not $retryWasRequired -or $script:taskSuitePosts.Count -ne 0) {
      throw "Publication workflow retried without an explicit same-run recovery decision"
    }

    Reset-TaskSuiteWorkflowMocks
    $script:taskSuiteRuns.Enqueue((New-TaskSuiteWorkflowRun "failure" 1))
    $script:taskSuiteRuns.Enqueue((New-TaskSuiteWorkflowRun "failure" 2))
    $secondFailureStopped = $false
    try {
      $null = Wait-ReleasePublicationWorkflow @workflowArgs -RetryFailedOnce
    } catch {
      $secondFailureStopped = $_.Exception.Message -like "*attempt 2 completed*failure*"
    }
    if (-not $secondFailureStopped -or $script:taskSuitePosts.Count -ne 1) {
      throw "Publication workflow recovery is not bounded to one retry"
    }

    $script:taskSuiteFallbackCandidate = "a" * 40
    $script:taskSuiteFallbackPrevious = "b" * 40
    $script:taskSuiteFallbackEvents = [System.Collections.Generic.List[string]]::new()
    $script:taskSuiteFallbackBranchReads = 0
    $script:taskSuiteFallbackPushArguments = @()
    function Get-PublicationRemoteTagCommit {
      param([string]$RepoRoot, [string]$Tag)
      [void]$script:taskSuiteFallbackEvents.Add("tag-read")
      return $null
    }
    function Get-PublicationRemoteBranchCommit {
      param([string]$RepoRoot, [string]$Branch)
      [void]$script:taskSuiteFallbackEvents.Add("branch-read")
      $script:taskSuiteFallbackBranchReads += 1
      if ($script:taskSuiteFallbackBranchReads -eq 1) {
        return $script:taskSuiteFallbackPrevious
      }
      return $script:taskSuiteFallbackCandidate
    }
    function Assert-PublicationFallbackAdvance {
      param(
        [string]$RepoRoot,
        [string]$Version,
        [string]$PreviousCandidate,
        [string]$CandidateSha
      )
      if ($PreviousCandidate -ne $script:taskSuiteFallbackPrevious -or
          $CandidateSha -ne $script:taskSuiteFallbackCandidate) {
        throw "Fallback recovery proof received the wrong candidate boundary"
      }
      [void]$script:taskSuiteFallbackEvents.Add("fallback-proof")
    }
    function Invoke-ReleasePublicationGit {
      param(
        [string]$RepoRoot,
        [string[]]$Arguments,
        [switch]$AllowFailure
      )
      $result = [pscustomobject]@{
        ExitCode = 0
        StdOut = ""
        StdErr = ""
        Output = ""
      }
      if ($Arguments[0] -eq "rev-parse") {
        $result.StdOut = $script:taskSuiteFallbackCandidate
        return $result
      }
      if ($Arguments[0] -eq "show-ref") {
        $result.ExitCode = 1
        return $result
      }
      if ($Arguments[0] -eq "update-ref") {
        [void]$script:taskSuiteFallbackEvents.Add("local-tag-update")
        return $result
      }
      if ($Arguments[0] -eq "push" -and $Arguments[-1] -like "*refs/tags/*") {
        [void]$script:taskSuiteFallbackEvents.Add("tag-push")
        $result.ExitCode = 1
        $result.Output = "tag push blocked"
        return $result
      }
      if ($Arguments[0] -eq "push" -and $Arguments[-1] -like "*refs/heads/release/*") {
        [void]$script:taskSuiteFallbackEvents.Add("fallback-push")
        $script:taskSuiteFallbackPushArguments = @($Arguments)
        return $result
      }
      $joinedArguments = $Arguments -join [char]32
      throw "Unexpected mocked Git call: $joinedArguments"
    }
    $fallbackResult = Invoke-ReleasePublicationTagPush `
      -RepoRoot $root `
      -Version "9.8.7" `
      -CandidateSha $script:taskSuiteFallbackCandidate
    $expectedLease = "--force-with-lease=refs/heads/release/v9.8.7:$($script:taskSuiteFallbackPrevious)"
    $proofIndex = $script:taskSuiteFallbackEvents.IndexOf("fallback-proof")
    $localTagIndex = $script:taskSuiteFallbackEvents.IndexOf("local-tag-update")
    $tagPushIndex = $script:taskSuiteFallbackEvents.IndexOf("tag-push")
    $fallbackPushIndex = $script:taskSuiteFallbackEvents.IndexOf("fallback-push")
    if ($fallbackResult.TriggerBranch -ne "release/v9.8.7" -or
        $fallbackResult.ExistingRun -ne $false -or
        $script:taskSuiteFallbackPushArguments -notcontains $expectedLease -or
        $proofIndex -lt 0 -or $localTagIndex -le $proofIndex -or
        $tagPushIndex -le $localTagIndex -or $fallbackPushIndex -le $tagPushIndex -or
        ($script:taskSuiteFallbackEvents | Where-Object { $_ -eq "fallback-proof" }).Count -ne 1 -or
        ($script:taskSuiteFallbackEvents | Where-Object { $_ -eq "tag-read" }).Count -lt 4) {
      throw "Fallback publication is not durably proved and CAS-bound before its only trigger push"
    }
    foreach ($buildScript in @(
      "native/publish-feed.sh",
      "native/publish-linux-feed-wsl.sh",
      "native/publish-update.ps1"
    )) {
      $buildText = Get-Content -LiteralPath (Join-Path $root $buildScript) -Raw
      if ($buildText -notmatch "HEAD\^\{commit\}" -or
          $buildText -match "head_subject|headSubject" -or
          [regex]::Matches($buildText, "cargo build --locked").Count -gt
            [regex]::Matches($buildText, "--target-dir").Count) {
        throw "Release build provenance is not bound directly to current HEAD in $buildScript"
      }
      if ($buildScript -eq "native/publish-update.ps1" -and
          ([regex]::Matches($buildText, "cargo build --locked").Count -gt
            [regex]::Matches($buildText, "--target ").Count -or
           $buildText -match "target\\release")) {
        throw "Native Windows release outputs are not pinned to the detected host target"
      }
    }
  '

  python3 - "$repo_root/.github/workflows/build.yml" <<'PY'
import pathlib
import sys

import yaml

workflow = yaml.safe_load(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
assert isinstance(workflow, dict)
jobs = workflow.get("jobs", {})

for name in (
    "windows-native-tests",
    "release-candidate",
    "windows-gnu-release-e2e",
    "publish-release",
):
    assert name in jobs, f"workflow job is missing: {name}"

publish = jobs["publish-release"]
assert publish.get("needs") == "release-candidate"
publish_checkout = next(
    step for step in publish.get("steps", [])
    if step.get("uses") == "actions/checkout@v4"
)
assert publish_checkout.get("with", {}).get("fetch-depth") == 0
publish_condition = publish.get("if", "")
assert "refs/tags/v" in publish_condition
assert "inputs.publish_release == true" in publish_condition
assert "refs/heads/release/v" in publish_condition
assert "verify_release_candidate" not in publish_condition
assert "refs/heads/verify/v" not in publish_condition
publish_text = repr(publish)
assert "cargo " not in publish_text
assert "test-share-lifecycle-e2e" not in publish_text
assert "POST" in publish_text and "/git/refs" in publish_text

candidate_steps = jobs["release-candidate"].get("steps", [])
checkout = next(
    step for step in candidate_steps
    if step.get("uses") == "actions/checkout@v4"
)
assert checkout.get("with", {}).get("fetch-depth") == 0
stage = next(
    step for step in candidate_steps
    if step.get("name") == "Verify and stage exact committed release candidate"
)
stage_text = stage.get("run", "")
assert "cargo " not in stage_text
assert "test-share-lifecycle-e2e" not in stage_text
assert "changed_count" in stage_text
assert "contains no release publication changes" in stage_text
upload = next(
    step for step in candidate_steps
    if step.get("uses") == "actions/upload-artifact@v4"
)
assert upload.get("with", {}).get("overwrite") is True
for step in candidate_steps:
    if "E2E" not in step.get("name", ""):
        continue
    condition = step.get("if", "")
    assert "inputs.verify_release_candidate == true" in condition
    assert "refs/heads/verify/v" in condition
    assert "refs/tags/v" not in condition
    assert "inputs.publish_release == true" not in condition
    assert "refs/heads/release/v" not in condition

windows_exact = jobs["windows-gnu-release-e2e"]
windows_condition = windows_exact.get("if", "")
assert "inputs.verify_release_candidate == true" in windows_condition
assert "refs/heads/verify/v" in windows_condition
assert "refs/tags/v" not in windows_condition
assert "inputs.publish_release == true" not in windows_condition
assert "refs/heads/release/v" not in windows_condition
windows_text = repr(windows_exact)
assert windows_text.count("-PeerSeBinary $se") == 2
assert "debug-peer" not in windows_text
PY

  grep -F 'cargo build --locked' "$repo_root/native/build-agent-bundles.sh" >/dev/null
  grep -F 'cargo build --locked' "$repo_root/native/publish-feed.sh" >/dev/null
  grep -F 'cargo build --locked' "$repo_root/native/publish-linux-feed-wsl.sh" >/dev/null
  grep -F 'cargo build --locked' "$repo_root/native/publish-update.ps1" >/dev/null

  local install_dry_run
  install_dry_run="$(
    SMART_EXPLORER_RELEASE_TAG=v9.8.7 \
    SMART_EXPLORER_REQUIRE_RELEASE_ASSETS=1 \
    SMART_EXPLORER_INSTALL_DIR="$root/release-check/install" \
    SMART_EXPLORER_BIN_DIR="$root/release-check/bin" \
      sh "$repo_root/install-linux.sh" --dry-run --cli-only 2>&1
  )"
  grep -F 'releases/download/v9.8.7' <<<"$install_dry_run" >/dev/null
  grep -F '(atomic rename)' <<<"$install_dry_run" >/dev/null
  if grep -F 'update_source.txt' <<<"$install_dry_run" >/dev/null; then
    echo "CLI-only install unexpectedly rewrites the desktop update source" >&2
    return 1
  fi

  set +e
  local no_build_path="$root/release-check/no-build-path"
  mkdir -p "$no_build_path"
  ln -s "$(command -v dirname)" "$no_build_path/dirname"
  ln -s "$(command -v uname)" "$no_build_path/uname"
  SMART_EXPLORER_RELEASE_LOCK_TOKEN= \
    PATH="$no_build_path" \
    "$BASH" "$repo_root/native/publish-feed.sh" \
      >"$root/direct-publish.stdout" 2>"$root/direct-publish.stderr"
  local direct_publish_status=$?
  set -e
  [[ $direct_publish_status -ne 0 ]]
  grep -F 'only run through native/publish-release-local.ps1' \
    "$root/direct-publish.stderr" >/dev/null
}

wait_request_inbox() {
  local client="$1"
  local deadline=$((SECONDS + 40))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request 2>/dev/null)" \
      && grep -Fqx $'pending_requests\t1' <<<"$value" \
      && grep -F $'delivery=received\tdecision=pending\tauthorization=inactive' \
        >/dev/null <<<"$value" \
      && grep -Fqx $'next\tse share request accept' <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "no received pending request appeared in the bare inbox for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_request_inbox_json() {
  local client="$1"
  local deadline=$((SECONDS + 45))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request --json 2>/dev/null)" \
      && jq -e '.count == 1 and (.requests | length) == 1 and .requests[0].direction == "incoming" and .requests[0].delivery.state == "received" and .requests[0].decision.state == "pending" and .requests[0].authorization.active == false' \
        >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "no single received pending JSON request appeared for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_empty_request_inbox() {
  local client="$1"
  local deadline=$((SECONDS + 45))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request --json 2>/dev/null)" \
      && jq -e '.count == 0 and (.requests | length) == 0' >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "pending inbox did not become empty for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_request_state() {
  local client="$1"
  local request_id="$2"
  local jq_filter="$3"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request show "$request_id" --json 2>/dev/null)" \
      && jq -e "$jq_filter" >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "request $request_id never satisfied: $jq_filter" >&2
  return 1
}

wait_exec_state() {
  local client="$1"
  local direction="$2"
  local state="$3"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share exec --json 2>/dev/null)" \
      && jq -e --arg direction "$direction" --arg state "$state" \
        '[.[] | select(.direction == $direction and .job.state == $state)] | length == 1' >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.1
  done
  echo "no $direction Exec reached $state for $client" >&2
  return 1
}

wait_exec_history() {
  local client="$1"
  local direction="$2"
  local state="$3"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share exec history --json 2>/dev/null)" \
      && jq -e --arg direction "$direction" --arg state "$state" \
        'any(.[]; .direction == $direction and .job.state == $state)' >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.1
  done
  echo "no $direction Exec history reached $state for $client" >&2
  return 1
}

wait_exec_history_id() {
  local client="$1"
  local direction="$2"
  local state="$3"
  local exec_id="$4"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share exec history --json 2>/dev/null)" \
      && jq -e --arg direction "$direction" --arg state "$state" --arg exec_id "$exec_id" \
        'any(.[]; .direction == $direction and .job.state == $state and .job.exec_id == $exec_id)' \
        >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.1
  done
  echo "Exec $exec_id did not reach $direction/$state history for $client" >&2
  return 1
}

wait_exec_unit_stopped() {
  local exec_id="$1"
  local unit="smart-explorer-exec-$exec_id.service"
  local deadline=$((SECONDS + 20))
  local active
  while [[ $SECONDS -lt $deadline ]]; do
    if [[ $(id -u) -eq 0 ]]; then
      active="$(systemctl is-active "$unit" 2>/dev/null || true)"
    else
      active="$(systemctl --user is-active "$unit" 2>/dev/null || true)"
    fi
    if [[ "$active" != active && "$active" != activating && "$active" != deactivating ]]; then
      return 0
    fi
    sleep 0.1
  done
  echo "Exec unit $unit remained active after worker death" >&2
  return 1
}

wait_cgroup_empty_or_gone() {
  local cgroup="$1"
  local deadline=$((SECONDS + 20))
  while [[ $SECONDS -lt $deadline ]]; do
    if [[ ! -e "$cgroup" ]] || grep -Fqx 'populated 0' "$cgroup/cgroup.events" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  echo "Exec cgroup remained populated: $cgroup" >&2
  return 1
}

wait_child() {
  local pid="$1"
  local timeout="$2"
  local deadline=$((SECONDS + timeout))
  while kill -0 "$pid" 2>/dev/null && [[ $SECONDS -lt $deadline ]]; do
    sleep 0.1
  done
  if kill -0 "$pid" 2>/dev/null; then
    ps -o pid,ppid,stat,wchan:32,etime,cmd -p "$pid" >&2 || true
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "child $pid did not exit within ${timeout}s" >&2
    return 1
  fi
  set +e
  wait "$pid"
  child_status=$?
  set -e
}

verify_release_transaction_scripts

prepare_client "$client_a"
prepare_client "$client_b"
prepare_client "$client_c"
prepare_client "$client_d"

signal_port=$((31000 + ($$ % 12000)))
relay_port=$((signal_port + 1))
dead_signal_port=$((signal_port + 2))
dead_relay_port=$((signal_port + 3))
working_signal="127.0.0.1:$signal_port"
dead_signal="127.0.0.1:$dead_signal_port"
working_relay="http://127.0.0.1:$relay_port"
dead_relay="http://127.0.0.1:$dead_relay_port"
fallback_signal_config="$dead_signal,$working_signal"
"$server_bin" "127.0.0.1:$signal_port" >"$server_log" 2>&1 &
server_pid=$!
sleep 0.5
kill -0 "$server_pid"

identity_b="$(run_client "$client_b" share identity --json)"
direct_code_b="$(jq -er '.direct_code' <<<"$identity_b")"

# Queue while the target is offline. The requester must report relay state,
# never peer receipt, until B durably receives the signed envelope.
run_client "$client_a" share configure --server "$fallback_signal_config" >/dev/null
stop_daemon "$client_a"
run_client "$client_a" share status --json >/dev/null
wait_relay_route "$client_a" "$working_relay" >/dev/null
add_output="$(run_client "$client_a" connections add-peer --code "$direct_code_b" --name Target --json)"
peer_selector="$(jq -er '.selector' <<<"$add_output")"
direct_endpoint="$(jq -er '.endpoint' <<<"$add_output")"
contact_id="${direct_endpoint#share://direct/}"
request_id="$(jq -er '.request_id' <<<"$add_output")"
[[ -n "$peer_selector" && -n "$contact_id" && "$direct_endpoint" == share://direct/* && -n "$request_id" ]]
jq -e '
  .request.request_id == .request_id and
  .request.direction == "outgoing" and
  (.request.delivery.state | type == "string") and
  (.request.relay | has("outcome")) and
  .request.decision.state == "pending" and
  .request.authorization.active == false and
  (.worker_refresh.state | type == "string") and
  (.worker_refresh | has("error"))
' >/dev/null <<<"$add_output"

wait_request_state "$client_a" "$request_id" '.relay.outcome == "target_offline" and .peer_receipt.request.state == "unconfirmed"' >/dev/null
shown="$(run_client "$client_a" share request show --json)"
jq -e --arg id "$request_id" '.request_id == $id' >/dev/null <<<"$shown"

run_client "$client_b" share configure --server "$working_signal" >/dev/null
stop_daemon "$client_b"
run_client "$client_b" share status --json >/dev/null
wait_relay_route "$client_b" "$working_relay" >/dev/null
retry="$(run_client "$client_a" share request retry --json)"
jq -e --arg id "$request_id" '.request.request_id == $id' >/dev/null <<<"$retry"

# The target discovers everything it needs from one bare inbox command. No
# request ID, device ID, or fingerprint is supplied out of band.
inbox="$(wait_request_inbox "$client_b")"
mapfile -t inbox_request_lines < <(awk -F '\t' '$1 == "pending_request"' <<<"$inbox")
[[ ${#inbox_request_lines[@]} -eq 1 ]]
IFS=$'\t' read -r inbox_kind inbox_request_id inbox_name inbox_device_id inbox_fingerprint \
  inbox_delivery inbox_decision inbox_authorization inbox_identity_conflict \
  <<<"${inbox_request_lines[0]}"
[[ "$inbox_kind" == pending_request && -n "$inbox_request_id" ]]
[[ "$inbox_name" == device_name=?* && "$inbox_device_id" == device_id=?* ]]
[[ "$inbox_fingerprint" == fingerprint=?* && "$inbox_delivery" == delivery=received ]]
[[ "$inbox_decision" == decision=pending && "$inbox_authorization" == authorization=inactive ]]
[[ "$inbox_identity_conflict" == identity_conflict=false ]]
[[ "$inbox_request_id" == "$request_id" ]]
wait_request_state "$client_a" "$request_id" '.delivery.state == "received" and .peer_receipt.request.state == "received"' >/dev/null

# Pending inbox survives a full target daemon restart.
stop_daemon "$client_b"
run_client "$client_b" share status --json >/dev/null
wait_request_state "$client_b" "$inbox_request_id" '.direction == "incoming" and .decision.state == "pending"' >/dev/null

# The requester is offline while B accepts. B retains and retries the signed
# decision; A applies it after restart and returns a signed decision receipt.
stop_daemon "$client_a"
accepted="$(run_client "$client_b" share request accept --json)"
jq -e '.request.decision.state == "accepted" and .request.authorization.active == true' >/dev/null <<<"$accepted"
run_client "$client_a" share status --json >/dev/null
wait_request_state "$client_a" "$request_id" '.decision.state == "accepted" and .authorization.active == true' >/dev/null
wait_request_state "$client_b" "$inbox_request_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null

grants="$(run_client "$client_b" share grants --json)"
jq -e '.grants | any(.authorization.active == true)' >/dev/null <<<"$grants"

# Deleting the signed authorization basis while its grant is active must be
# refused. The user first revokes, waits for the peer receipt, then may delete
# the now-inactive history without supplying a hidden selector.
set +e
run_client "$client_b" share request delete --json \
  >"$root/delete-active.stdout" 2>"$root/delete-active.stderr"
delete_active_status=$?
set -e
[[ $delete_active_status -ne 0 ]]
run_client "$client_b" share request show --json >/dev/null

# An accepted authorization is operational, not merely a UI flag.
run_client "$client_a" ls "$direct_endpoint" >/dev/null

# Exec remains a separate exact-device grant. The target discovers the only
# accepted device from its own CLI; no request/device/fingerprint fixture is
# passed to the grant or execution commands.
exec_grants="$(run_client "$client_b" share grants exec --json)"
jq -e 'length == 1 and .[0].enabled == false' >/dev/null <<<"$exec_grants"
enabled="$(run_client "$client_b" share grants exec enable --yes --json)"
jq -e '.persisted == true and .applied == true' >/dev/null <<<"$enabled"

# Older saved connection profiles can retain a route after signaling changes.
# With signaling intentionally unreachable, prove that expired and implausibly
# far-future routes fail closed, then prove that a still-current legacy route
# with an unusable relay alias recovers through this node's configured relay.
stop_daemon "$client_a"
profile_a="$(single_client_file "$client_a" share_profiles.json)"
server_config_a="$(single_client_file "$client_a" share_server.txt)"
cp "$profile_a" "$root/profile-a-before-route-recovery.json"
printf '%s' "$dead_signal" >"$server_config_a"
printf '%s' "$working_relay" >"$client_a/relay-url"

rewrite_direct_presence "$profile_a" "$contact_id" "$dead_relay" 1
run_client "$client_a" share status --json >/dev/null
set +e
run_client "$client_a" exec -- true \
  >"$root/expired-exec.stdout" 2>"$root/expired-exec.stderr"
expired_exec_status=$?
run_client "$client_a" ls "$direct_endpoint" \
  >"$root/expired-ls.stdout" 2>"$root/expired-ls.stderr"
expired_ls_status=$?
set -e
[[ $expired_exec_status -ne 0 && $expired_ls_status -ne 0 ]]
grep -F 'no ready Exec peer was found' "$root/expired-exec.stderr" >/dev/null

stop_daemon "$client_a"
too_far_future=$(( $(date +%s) + 3600 ))
rewrite_direct_presence "$profile_a" "$contact_id" "$dead_relay" "$too_far_future"
run_client "$client_a" share status --json >/dev/null
set +e
run_client "$client_a" exec -- true \
  >"$root/future-exec.stdout" 2>"$root/future-exec.stderr"
future_exec_status=$?
run_client "$client_a" ls "$direct_endpoint" \
  >"$root/future-ls.stdout" 2>"$root/future-ls.stderr"
future_ls_status=$?
set -e
[[ $future_exec_status -ne 0 && $future_ls_status -ne 0 ]]
grep -F 'no ready Exec peer was found' "$root/future-exec.stderr" >/dev/null

stop_daemon "$client_a"
current_legacy_expiry=$(( $(date +%s) + 600 ))
rewrite_direct_presence "$profile_a" "$contact_id" "$dead_relay" "$current_legacy_expiry"
run_client "$client_a" share status --json >/dev/null
wait_relay_route "$client_a" "$working_relay" disconnected >/dev/null
jq -e --arg contact_id "$contact_id" --arg relay "$dead_relay" '
  [.direct_contacts[] |
    select(.id == $contact_id and
      .presence.relay_url == $relay and
      (.presence.candidates | length) == 0)] | length == 1
' "$profile_a" >/dev/null
run_client "$client_a" ls "$direct_endpoint" >/dev/null
legacy_transport="$(run_client "$client_a" share status --json)"
jq -e 'any(.events[]?; contains(" via relay "))' \
  >/dev/null <<<"$legacy_transport"
legacy_exec="$(run_client "$client_a" exec "$direct_endpoint" -- sh -c 'printf LEGACY_RELAY_OK')"
[[ "$legacy_exec" == LEGACY_RELAY_OK ]]

stop_daemon "$client_a"
rm -f "$client_a/relay-url"
printf '%s' "$fallback_signal_config" >"$server_config_a"
run_client "$client_a" share status --json >/dev/null
wait_relay_route "$client_a" "$working_relay" >/dev/null

# Learn the target home from the remote shell itself; every later target path
# is derived from this earlier CLI output, never from the test harness layout.
remote_home="$(run_client "$client_a" exec -- sh -c 'printf %s "$HOME"')"
[[ "$remote_home" == /* ]]

# A healthy silent command outlives the application heartbeat deadline.
silent_started=$SECONDS
silent_output="$(run_client "$client_a" exec -- sh -c 'sleep 25; printf LINUX_HEARTBEAT_OK')"
[[ "$silent_output" == LINUX_HEARTBEAT_OK ]]
[[ $((SECONDS - silent_started)) -ge 24 ]]

# Literal argv, arbitrary binary stdin/stdout, stderr, and a non-zero remote
# exit code cross the actual daemon IPC and Iroh Exec protocol.
exec_input="$root/exec-input.bin"
exec_output="$root/exec-output.bin"
exec_error="$root/exec-error.txt"
printf 'binary\000stdin\377\200\n' >"$exec_input"
set +e
run_client "$client_a" exec -- sh -c 'cat; printf "remote-stderr\n" >&2; exit 7' \
  <"$exec_input" >"$exec_output" 2>"$exec_error"
exec_status=$?
set -e
[[ $exec_status -eq 7 ]]
cmp "$exec_input" "$exec_output"
grep -Fqx 'remote-stderr' "$exec_error"
wait_exec_history "$client_a" outgoing exited >/dev/null
wait_exec_history "$client_b" incoming exited >/dev/null

# An explicit output cap truncates bytes without losing the terminal result.
set +e
run_client "$client_a" exec --max-output 5 -- sh -c 'printf 1234567890' \
  </dev/null >"$root/limited.stdout" 2>"$root/limited.stderr"
limited_status=$?
set -e
[[ $limited_status -eq 0 ]]
[[ "$(cat "$root/limited.stdout")" == 12345 ]]
grep -F 'remote output was truncated' "$root/limited.stderr" >/dev/null

# Timeout must kill a stubborn descendant before it can leave a delayed marker.
timeout_marker="$remote_home/timed-out-exec-must-not-run"
set +e
run_client "$client_a" exec --timeout 1 -- sh -c \
  '(sleep 3; touch "$1") & wait' sh "$timeout_marker" \
  </dev/null >"$root/timeout.stdout" 2>"$root/timeout.stderr"
timeout_status=$?
set -e
[[ $timeout_status -eq 124 ]]
wait_exec_history "$client_a" outgoing timed_out >/dev/null
wait_exec_history "$client_b" incoming timed_out >/dev/null
sleep 3.5
[[ ! -e "$timeout_marker" ]]
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$timeout_marker" >/dev/null

# The target can find and cancel the sole incoming active command without an
# externally supplied Exec ID. Both endpoints must converge on Cancelled.
cancel_stdout="$root/cancel.stdout"
cancel_stderr="$root/cancel.stderr"
cancel_marker="$remote_home/cancelled-exec-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 5; touch "$1") & wait' sh "$cancel_marker" \
  </dev/null >"$cancel_stdout" 2>"$cancel_stderr" &
cancel_pid=$!
wait_exec_state "$client_a" outgoing running >/dev/null
wait_exec_state "$client_b" incoming running >/dev/null
cancelled="$(run_client "$client_b" share exec cancel --json)"
jq -e '.cancel_requested == true and (.exec_id | length == 32)' >/dev/null <<<"$cancelled"
wait_child "$cancel_pid" 30
[[ $child_status -eq 130 ]]
wait_exec_history "$client_a" outgoing cancelled >/dev/null
wait_exec_history "$client_b" incoming cancelled >/dev/null
sleep 5.5
[[ ! -e "$cancel_marker" ]]
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$cancel_marker" >/dev/null


# Killing only the foreground CLI closes local IPC; the worker must cancel the
# exact remote cgroup, and its delayed descendant must never reach the marker.
disconnect_marker="$remote_home/disconnected-cli-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 3; touch "$1") & wait' sh "$disconnect_marker" \
  </dev/null >"$root/disconnect.stdout" 2>"$root/disconnect.stderr" &
disconnect_pid=$!
disconnect_jobs="$(wait_exec_state "$client_b" incoming running)"
disconnect_id="$(jq -er '[.[] | select(.direction == "incoming" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous incoming Exec") end' <<<"$disconnect_jobs")"
kill -KILL "$disconnect_pid"
wait "$disconnect_pid" 2>/dev/null || true
wait_exec_history_id "$client_b" incoming cancelled "$disconnect_id" >/dev/null
sleep 3.5
[[ ! -e "$disconnect_marker" ]]
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$disconnect_marker" >/dev/null

# A hard target-worker crash must rely on kernel/systemd containment, not on a
# cooperative remote Cancel. The transient unit and owner-only socket must go
# away before the delayed child could write its marker.
crash_marker="$remote_home/crashed-worker-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 5; touch "$1") & wait' sh "$crash_marker" \
  </dev/null >"$root/crash.stdout" 2>"$root/crash.stderr" &
crash_cli_pid=$!
crash_outgoing_jobs="$(wait_exec_state "$client_a" outgoing running)"
crash_incoming_jobs="$(wait_exec_state "$client_b" incoming running)"
crash_outgoing_id="$(jq -er '[.[] | select(.direction == "outgoing" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous outgoing Exec") end' <<<"$crash_outgoing_jobs")"
crash_incoming_id="$(jq -er '[.[] | select(.direction == "incoming" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous incoming Exec") end' <<<"$crash_incoming_jobs")"
[[ "$crash_outgoing_id" == "$crash_incoming_id" ]]
crash_unit="smart-explorer-exec-$crash_incoming_id.service"
if [[ $(id -u) -eq 0 ]]; then
  crash_control_group="$(systemctl show "$crash_unit" -p ControlGroup --value)"
else
  crash_control_group="$(systemctl --user show "$crash_unit" -p ControlGroup --value)"
fi
[[ "$crash_control_group" == /* ]]
crash_cgroup="/sys/fs/cgroup${crash_control_group}"
[[ -f "$crash_cgroup/cgroup.events" ]]
mapfile -t target_daemons < <(daemon_pids "$client_b")
[[ ${#target_daemons[@]} -eq 1 ]]
kill -KILL "${target_daemons[0]}"
wait_child "$crash_cli_pid" 30
[[ $child_status -eq 125 ]]
wait_exec_history_id "$client_a" outgoing disconnected "$crash_outgoing_id" >/dev/null
wait_exec_unit_stopped "$crash_incoming_id"
wait_cgroup_empty_or_gone "$crash_cgroup"
uid="$(id -u)"
[[ ! -e "/run/user/$uid/smart-explorer-exec/$crash_incoming_id.sock" ]]
[[ ! -e "/tmp/smart-explorer-runtime-$uid/smart-explorer-exec/$crash_incoming_id.sock" ]]
sleep 5.5
[[ ! -e "$crash_marker" ]]
run_client "$client_b" share status --json >/dev/null
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$crash_marker" >/dev/null

# A hard requester-worker crash closes local IPC immediately, but the target
# must independently notice the missing authenticated Pings and kill the whole
# remote process tree. The command itself has no runtime limit.
requester_crash_marker="$remote_home/crashed-requester-worker-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 30; touch "$1") & wait' sh "$requester_crash_marker" \
  </dev/null >"$root/requester-crash.stdout" 2>"$root/requester-crash.stderr" &
requester_crash_cli_pid=$!
requester_outgoing_jobs="$(wait_exec_state "$client_a" outgoing running)"
requester_incoming_jobs="$(wait_exec_state "$client_b" incoming running)"
requester_outgoing_id="$(jq -er '[.[] | select(.direction == "outgoing" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous outgoing Exec") end' <<<"$requester_outgoing_jobs")"
requester_incoming_id="$(jq -er '[.[] | select(.direction == "incoming" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous incoming Exec") end' <<<"$requester_incoming_jobs")"
[[ "$requester_outgoing_id" == "$requester_incoming_id" ]]
mapfile -t requester_daemons < <(daemon_pids "$client_a")
[[ ${#requester_daemons[@]} -eq 1 ]]
kill -KILL "${requester_daemons[0]}"
wait_child "$requester_crash_cli_pid" 15
[[ $child_status -eq 125 ]]
wait_exec_history_id "$client_b" incoming disconnected "$requester_incoming_id" >/dev/null
# Wait beyond the descendant's own 30-second delay. Checking earlier would
# pass even if containment had leaked the process tree.
sleep 31
[[ ! -e "$requester_crash_marker" ]]
run_client "$client_a" share status --json >/dev/null
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$requester_crash_marker" >/dev/null

# Disabling the exact Exec grant terminates an already running process and
# prevents a later command from starting any payload.
revoke_marker="$remote_home/revoked-exec-must-not-run"
revoke_child_marker="$remote_home/revoked-active-child-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 5; touch "$1") & wait' sh "$revoke_child_marker" \
  </dev/null >"$root/revoke.stdout" 2>"$root/revoke.stderr" &
revoke_pid=$!
wait_exec_state "$client_b" incoming running >/dev/null
disabled="$(run_client "$client_b" share grants exec disable --json)"
jq -e '.persisted == true and .applied == true' >/dev/null <<<"$disabled"
wait_child "$revoke_pid" 30
[[ $child_status -eq 125 ]]
wait_exec_history "$client_a" outgoing revoked >/dev/null
wait_exec_history "$client_b" incoming revoked >/dev/null
sleep 5.5
[[ ! -e "$revoke_child_marker" ]]
set +e
run_client "$client_a" exec -- sh -c "touch '$revoke_marker'" \
  >"$root/denied.stdout" 2>"$root/denied.stderr"
denied_status=$?
set -e
[[ $denied_status -eq 125 ]]
[[ ! -e "$revoke_marker" ]]

revoked="$(run_client "$client_b" share grants revoke --json)"
jq -e '.request.decision.state == "revoked" and .request.authorization.active == false' >/dev/null <<<"$revoked"
wait_request_state "$client_a" "$request_id" '.decision.state == "revoked" and .authorization.active == false' >/dev/null
wait_request_state "$client_b" "$inbox_request_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null
deleted="$(run_client "$client_b" share request delete --json)"
jq -e --arg id "$inbox_request_id" '.action == "deleted" and .request_id == $id and .persisted == true' >/dev/null <<<"$deleted"

if run_client "$client_a" ls "$direct_endpoint" >/dev/null 2>&1; then
  echo "revoked direct authorization still allowed file access" >&2
  exit 1
fi

# Peer removal auto-selects the only peer without requiring a hidden selector.
connections="$(run_client "$client_a" connections list --json)"
jq -e --arg selector "$peer_selector" --arg endpoint "$direct_endpoint" \
  'length == 1 and .[0].kind == "direct" and .[0].selector == $selector and .[0].endpoint == $endpoint' \
  >/dev/null <<<"$connections"
run_client "$client_a" connections remove-peer >/dev/null
after_remove="$(run_client "$client_a" connections list --json)"
jq -e 'all(.[]; .kind != "direct")' >/dev/null <<<"$after_remove"

# A fresh third device avoids inheriting B's intentionally revoked grant and
# exercises bare rejection plus rejected-history deletion from a true pending
# state.
identity_c="$(run_client "$client_c" share identity --json)"
direct_code_c="$(jq -er '.direct_code' <<<"$identity_c")"
run_client "$client_c" share configure --server "$working_signal" >/dev/null
stop_daemon "$client_c"
run_client "$client_c" share status --json >/dev/null
wait_relay_route "$client_c" "$working_relay" >/dev/null
reject_add="$(run_client "$client_a" connections add-peer --code "$direct_code_c" --name RejectTarget --json)"
reject_request_id="$(jq -er '.request_id' <<<"$reject_add")"
reject_inbox="$(wait_request_inbox_json "$client_c")"
reject_inbox_id="$(jq -er '.requests[0].request_id' <<<"$reject_inbox")"
[[ "$reject_inbox_id" == "$reject_request_id" ]]
wait_request_state "$client_a" "$reject_request_id" '.delivery.state == "received" and .peer_receipt.request.state == "received"' >/dev/null
rejected="$(run_client "$client_c" share request reject --json)"
jq -e --arg id "$reject_inbox_id" '.request.request_id == $id and .request.decision.state == "rejected" and .request.authorization.active == false' >/dev/null <<<"$rejected"
wait_request_state "$client_a" "$reject_request_id" '.decision.state == "rejected" and .authorization.active == false' >/dev/null
wait_request_state "$client_c" "$reject_inbox_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null
run_client "$client_c" share request delete --json >/dev/null
run_client "$client_a" connections remove-peer >/dev/null

# A fourth fresh device supplies a genuinely pending request. Two full worker
# restarts prove the local dismissal tombstone remains durable and hidden.
identity_d="$(run_client "$client_d" share identity --json)"
direct_code_d="$(jq -er '.direct_code' <<<"$identity_d")"
run_client "$client_d" share configure --server "$working_signal" >/dev/null
stop_daemon "$client_d"
run_client "$client_d" share status --json >/dev/null
wait_relay_route "$client_d" "$working_relay" >/dev/null
pending_add="$(run_client "$client_a" connections add-peer --code "$direct_code_d" --name TombstoneTarget --json)"
pending_request_id="$(jq -er '.request_id' <<<"$pending_add")"
pending_inbox="$(wait_request_inbox_json "$client_d")"
pending_inbox_id="$(jq -er '.requests[0].request_id' <<<"$pending_inbox")"
[[ "$pending_inbox_id" == "$pending_request_id" ]]
pending_deleted="$(run_client "$client_d" share request delete --json)"
jq -e --arg id "$pending_inbox_id" '.action == "deleted" and .request_id == $id and .persisted == true' >/dev/null <<<"$pending_deleted"
stop_daemon "$client_d"
run_client "$client_d" share status --json >/dev/null
wait_empty_request_inbox "$client_d" >/dev/null
stop_daemon "$client_d"
run_client "$client_d" share status --json >/dev/null
wait_empty_request_inbox "$client_d" >/dev/null
run_client "$client_a" connections remove-peer >/dev/null

echo "tracked Share lifecycle E2E passed: $request_id"
