param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [string]$BeforeServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$AfterServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$EnvFile,

    [Parameter(Mandatory = $true)]
    [string]$BeforeJobsDirectoryName,

    [Parameter(Mandatory = $true)]
    [string]$AfterJobsDirectoryName,

    [Parameter(Mandatory = $true)]
    [int]$PrelaunchPid,

    [ValidateRange(1, 4)]
    [int]$MaxParallelPairs = 4
)

$ErrorActionPreference = 'Stop'

$pairRunner = Join-Path $PSScriptRoot 'run-terminal-bench-paired.ps1'
$parallelRunner = Join-Path $PSScriptRoot 'run-terminal-bench-paired-parallel.ps1'
$completionManifest = Join-Path $PlanRoot 'terminal-bench-prelaunch-completion-20260827.json'
$baseBefore = Join-Path $PlanRoot $BeforeJobsDirectoryName
$baseAfter = Join-Path $PlanRoot $AfterJobsDirectoryName
$providerMarkers = @(
    'provider request failed',
    'provider stream returned an error',
    'error sending request',
    'error decoding response body',
    'upstream_error'
)

foreach ($path in @($pairRunner, $parallelRunner, $BeforeServerBinary, $AfterServerBinary, $EnvFile)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required terminal recovery input is missing: $path"
    }
}

function Get-LatestTaskResult {
    param([string]$Task, [string]$JobsDirectory)

    $expectedTask = "terminal-bench/$Task"
    foreach ($result in @(Get-ChildItem -LiteralPath $JobsDirectory -Recurse -File -Filter result.json -ErrorAction SilentlyContinue | Sort-Object LastWriteTimeUtc -Descending)) {
        try {
            $payload = Get-Content -LiteralPath $result.FullName -Raw | ConvertFrom-Json
        } catch {
            continue
        }
        if ($payload.task_name -eq $expectedTask) {
            return $payload
        }
    }
    return $null
}

function Test-UsableSnapshot {
    param([string]$Task, [ValidateSet('before', 'after')][string]$Snapshot)

    $jobs = if ($Snapshot -eq 'before') { $baseBefore } else { $baseAfter }
    $payload = Get-LatestTaskResult -Task $Task -JobsDirectory $jobs
    if ($null -eq $payload) {
        return $false
    }
    $turnStatus = $payload.agent_result.metadata.turnStatus
    if ($turnStatus -isnot [string] -or -not $turnStatus) {
        return $false
    }
    $turnError = $payload.agent_result.metadata.turnError
    if ($turnError -is [string]) {
        $normalized = $turnError.ToLowerInvariant()
        if (@($providerMarkers | Where-Object { $normalized.Contains($_) }).Count -gt 0) {
            return $false
        }
    }
    return $true
}

function Invoke-SnapshotRetry {
    param([string]$Task, [ValidateSet('before', 'after')][string]$Snapshot)

    $arguments = @{
        PlanRoot = $PlanRoot
        BeforeServerBinary = $BeforeServerBinary
        AfterServerBinary = $AfterServerBinary
        EnvFile = $EnvFile
        BeforeJobsDirectoryName = "$BeforeJobsDirectoryName\\$Task"
        AfterJobsDirectoryName = "$AfterJobsDirectoryName\\$Task"
        RunLogFileName = "terminal-bench-$Task-$Snapshot-provider-retry-20260827.run-log.jsonl"
        OnlyPairTask = $Task
        OnlySnapshot = $Snapshot
    }
    & $pairRunner @arguments
}

$prelaunch = Get-Process -Id $PrelaunchPid -ErrorAction SilentlyContinue
if ($null -ne $prelaunch) {
    $prelaunch.WaitForExit()
}

# A provider failure is infrastructure noise rather than an agent-quality
# observation. Keep every usable side, and re-run only the failed snapshot.
$initialTasks = @('bun-sourcemap-leak', 'retro-console-soc', 'roy-polymorph-cn')
foreach ($task in $initialTasks) {
    foreach ($snapshot in @('before', 'after')) {
        if (-not (Test-UsableSnapshot -Task $task -Snapshot $snapshot)) {
            Invoke-SnapshotRetry -Task $task -Snapshot $snapshot
        }
        if (-not (Test-UsableSnapshot -Task $task -Snapshot $snapshot)) {
            throw "Provider recovery did not produce a usable result for $task/$snapshot."
        }
    }
}

# The first dispatcher stopped after the provider failure. Start at the first
# untouched task so already valid work is not repeated or charged again.
$resumeArguments = @{
    PlanRoot = $PlanRoot
    BeforeServerBinary = $BeforeServerBinary
    AfterServerBinary = $AfterServerBinary
    EnvFile = $EnvFile
    BeforeJobsDirectoryName = $BeforeJobsDirectoryName
    AfterJobsDirectoryName = $AfterJobsDirectoryName
    MaxParallelPairs = $MaxParallelPairs
    StartRemainingTask = 'data-anonymization'
}
$resumeSucceeded = $false
for ($attempt = 0; $attempt -lt 3 -and -not $resumeSucceeded; $attempt++) {
    $attemptArguments = @{}
    foreach ($entry in $resumeArguments.GetEnumerator()) {
        $attemptArguments[$entry.Key] = $entry.Value
    }
    # Missing snapshots and provider rejections are retried; every other
    # terminal result is reused. This also makes a restarted coordinator safe
    # after a partially completed dispatcher.
    $attemptArguments.RetryProviderFailures = $true
    try {
        & $parallelRunner @attemptArguments
        $resumeSucceeded = $true
    } catch {
        if ($attempt -eq 2) {
            throw
        }
    }
}

$resumeManifestPath = Join-Path $PlanRoot 'terminal-bench-parallel-manifest.json'
if (-not (Test-Path -LiteralPath $resumeManifestPath)) {
    throw "Terminal resume dispatcher did not write its manifest: $resumeManifestPath"
}
$resumeManifest = Get-Content -LiteralPath $resumeManifestPath -Raw | ConvertFrom-Json
$resumeCompleted = @($resumeManifest.completed)
$resumePending = @($resumeManifest.pendingSkippedAfterFailure)
if ($resumeCompleted.Count -ne 9 -or $resumePending.Count -ne 0 -or @($resumeCompleted | Where-Object { [int]$_.exitCode -ne 0 }).Count -ne 0) {
    throw 'Terminal resume dispatcher was incomplete or failed.'
}

$completion = [ordered]@{
    schemaVersion = 1
    maxParallelPairs = $MaxParallelPairs
    completed = @(
        $initialTasks | ForEach-Object { [ordered]@{ task = $_; exitCode = 0; recoveredProviderFailure = ($_ -eq 'roy-polymorph-cn') } }
    ) + $resumeCompleted
    pendingSkippedAfterFailure = @()
}
$completion | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $completionManifest -Encoding utf8
