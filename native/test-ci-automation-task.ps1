$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptRoot
$wrapperPath = Join-Path $scriptRoot "publish-release-local.ps1"
$publicationPath = Join-Path $scriptRoot "release-publication.ps1"
$workflowPaths = @(
    (Join-Path $repoRoot ".github/workflows/build.yml"),
    (Join-Path $repoRoot ".github/workflows/share-remote-task.yml")
)

function Assert-Task([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

. $publicationPath

$assetExpectationRoot = Join-Path (
    [System.IO.Path]::GetTempPath()
) "smart-explorer-publication-task-$([guid]::NewGuid().ToString('N'))"
[void](New-Item -ItemType Directory -Path $assetExpectationRoot)
try {
    $versionPath = Join-Path $assetExpectationRoot "version.txt"
    [System.IO.File]::WriteAllBytes(
        $versionPath,
        [System.Text.UTF8Encoding]::new($false).GetBytes("9.8.7`r`n")
    )
    $versionExpectation = Get-PublicationExpectedReleaseAsset `
        -Asset ([pscustomobject]@{
            LocalPath = $versionPath
            PublishedName = "version.txt"
        }) `
        -Version "9.8.7"
    $canonicalVersionPath = Join-Path $assetExpectationRoot "canonical-version.txt"
    [System.IO.File]::WriteAllBytes(
        $canonicalVersionPath,
        [System.Text.UTF8Encoding]::new($false).GetBytes("9.8.7`n")
    )
    Assert-Task ($versionExpectation.Size -eq 6) `
        "published version.txt expectation must use canonical LF bytes"
    Assert-Task (
        $versionExpectation.Sha256 -eq (Get-PublicationSha256 $canonicalVersionPath)
    ) "published version.txt expectation must hash the canonical Git representation"

    $binaryPath = Join-Path $assetExpectationRoot "payload.bin"
    [System.IO.File]::WriteAllBytes($binaryPath, [byte[]]@(0, 1, 2, 255))
    $binaryExpectation = Get-PublicationExpectedReleaseAsset `
        -Asset ([pscustomobject]@{
            LocalPath = $binaryPath
            PublishedName = "payload.bin"
        }) `
        -Version "9.8.7"
    Assert-Task ($binaryExpectation.Size -eq 4) `
        "binary release asset expectation must retain the exact file size"
    Assert-Task (
        $binaryExpectation.Sha256 -eq (Get-PublicationSha256 $binaryPath)
    ) "binary release asset expectation must retain the exact file digest"
} finally {
    Remove-Item -LiteralPath $assetExpectationRoot -Recurse -Force -ErrorAction SilentlyContinue
}

$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $wrapperPath,
    [ref]$tokens,
    [ref]$parseErrors
)
Assert-Task ($parseErrors.Count -eq 0) "publish-release-local.ps1 must parse without errors"
$waitFunction = $ast.Find({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq "Wait-GitHubActionsPublicationWorkflow"
}, $true)
Assert-Task ($null -ne $waitFunction) "publication wait function was not found"
. ([scriptblock]::Create($waitFunction.Extent.Text))

$script:workflowFile = "build.yml"
$candidateSha = "1234567890abcdef1234567890abcdef12345678"
$triggerBranch = "v9.8.7"
$title = "Publish release candidate $candidateSha"
$script:responses = [System.Collections.Generic.Queue[object]]::new()
$script:delayCalls = 0
$script:postCalls = 0

function New-TaskRun(
    [string]$Branch,
    [string]$Sha,
    [string]$Status,
    [string]$Conclusion = ""
) {
    return [pscustomobject]@{
        event = "workflow_dispatch"
        head_sha = $Sha
        head_branch = $Branch
        path = ".github/workflows/build.yml"
        display_title = $title
        status = $Status
        conclusion = $Conclusion
        run_attempt = 1
        html_url = "https://example.invalid/actions/runs/42"
    }
}

function Invoke-GitHubGet([string]$Path) {
    Assert-Task ($Path -eq "actions/runs/42") "waiter queried an unexpected run"
    Assert-Task ($script:responses.Count -gt 0) "waiter exhausted mocked run responses"
    return $script:responses.Dequeue()
}

function Wait-ReleasePublicationDelay([datetimeoffset]$Deadline) {
    $script:delayCalls += 1
    return $true
}

function Invoke-ReleasePublicationGitHubPost {
    $script:postCalls += 1
}

function Reset-TaskResponses {
    $script:responses.Clear()
    $script:delayCalls = 0
    $script:postCalls = 0
}

$deadline = [datetimeoffset]::UtcNow.AddMinutes(5)

Reset-TaskResponses
$script:responses.Enqueue((New-TaskRun "main" $candidateSha "queued"))
$script:responses.Enqueue((New-TaskRun $triggerBranch $candidateSha "completed" "success"))
$result = Wait-GitHubActionsPublicationWorkflow `
    -RunId 42 `
    -CandidateSha $candidateSha `
    -TriggerBranch $triggerBranch `
    -AllowInitialBindingPropagation `
    -Deadline $deadline
Assert-Task ($result.conclusion -eq "success") "initial metadata propagation was not tolerated"
Assert-Task ($script:delayCalls -eq 1) "initial metadata propagation should wait exactly once"

Reset-TaskResponses
$script:responses.Enqueue((New-TaskRun "main" $candidateSha "completed" "success"))
$completedMismatchFailed = $false
try {
    Wait-GitHubActionsPublicationWorkflow `
        -RunId 42 `
        -CandidateSha $candidateSha `
        -TriggerBranch $triggerBranch `
        -AllowInitialBindingPropagation `
        -Deadline $deadline
} catch {
    $completedMismatchFailed = $_.Exception.Message -like "*is not bound*"
}
Assert-Task $completedMismatchFailed "a completed run with the wrong binding must fail closed"

Reset-TaskResponses
$script:responses.Enqueue((New-TaskRun $triggerBranch $candidateSha "in_progress"))
$script:responses.Enqueue((New-TaskRun "main" $candidateSha "in_progress"))
$postBindingDriftFailed = $false
try {
    Wait-GitHubActionsPublicationWorkflow `
        -RunId 42 `
        -CandidateSha $candidateSha `
        -TriggerBranch $triggerBranch `
        -AllowInitialBindingPropagation `
        -Deadline $deadline
} catch {
    $postBindingDriftFailed = $_.Exception.Message -like "*is not bound*"
}
Assert-Task $postBindingDriftFailed "binding drift after an exact observation must fail closed"

Reset-TaskResponses
$script:responses.Enqueue((New-TaskRun "main" $candidateSha "queued"))
$existingMismatchFailed = $false
try {
    Wait-GitHubActionsPublicationWorkflow `
        -RunId 42 `
        -CandidateSha $candidateSha `
        -TriggerBranch $triggerBranch `
        -Deadline $deadline
} catch {
    $existingMismatchFailed = $_.Exception.Message -like "*is not bound*"
}
Assert-Task $existingMismatchFailed "an existing mismatched run must fail without a propagation grace period"

$wrapperText = [System.IO.File]::ReadAllText($wrapperPath)
Assert-Task (
    $wrapperText.Contains('-AllowInitialBindingPropagation:(-not $existingDispatch)')
) "only a freshly dispatched publication run should receive the binding propagation grace period"
foreach ($workflowPath in $workflowPaths) {
    $workflowText = [System.IO.File]::ReadAllText($workflowPath)
    Assert-Task (
        -not [regex]::IsMatch($workflowText, 'actions/checkout@v(?!7\b)\d+')
    ) "an outdated checkout runtime remains in $workflowPath"
    Assert-Task ($workflowText.Contains("actions/checkout@v7")) "current checkout runtime is missing from $workflowPath"
}
$buildWorkflow = [System.IO.File]::ReadAllText($workflowPaths[0])
Assert-Task (
    $buildWorkflow.Contains("endsWith(github.event.head_commit.message, '[task candidate]') == false")
) "ordinary push builds do not exclude the exact task-suite candidate"

$releaseGuide = [System.IO.File]::ReadAllText((Join-Path $repoRoot "docs/RELEASING.md"))
Assert-Task (
    $releaseGuide.Contains("compilation, tests, packaging, and publication execute on GitHub-hosted runners")
) "release guide does not make the GitHub-hosted path authoritative"
Assert-Task (
    $releaseGuide.Contains("allows up to two minutes for that initial metadata to settle")
) "release guide does not document the bounded metadata propagation wait"
Assert-Task (
    [regex]::IsMatch($releaseGuide, 'head\s+commit ends in `\[task candidate\]`')
) "release guide does not document single-suite candidate routing"
$readme = [System.IO.File]::ReadAllText((Join-Path $repoRoot "README.md"))
Assert-Task (
    $readme.Contains("Build, Tests, Paketierung und") -and
        $readme.Contains("bis zu zwei Minuten")
) "README does not summarize remote-only execution and bounded run binding"

Write-Host "CI automation task checks passed"
