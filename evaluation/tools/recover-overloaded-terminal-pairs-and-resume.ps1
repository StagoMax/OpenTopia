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
    [int]$InitialControllerPid,

    [ValidateRange(60, 900)]
    [int]$CooldownSeconds = 180,

    [ValidateRange(1, 4)]
    [int]$MaxParallelPairs = 2
)

$ErrorActionPreference = 'Stop'

$pairRunner = Join-Path $PSScriptRoot 'run-terminal-bench-paired.ps1'
$continuation = Join-Path $PSScriptRoot 'continue-public-benchmarks-after-pilot.ps1'
$controlLog = Join-Path $PlanRoot 'recover-overloaded-terminal-pairs-20260827.jsonl'
$currentBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-quota-restored-par2-20260827'
$currentAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-quota-restored-par2-20260827'
$rsBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-overload-rs-20260827'
$rsAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-overload-rs-20260827'
$distributedBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-overload-distributed-20260827'
$distributedAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-overload-distributed-20260827'
$remainingBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-after-overload-par2-20260827'
$remainingAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-after-overload-par2-20260827'

foreach ($path in @($pairRunner, $continuation, $BeforeServerBinary, $AfterServerBinary, $EnvFile)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required recovery input is missing: $path"
    }
}

function Write-ControlEvent {
    param([string]$Stage, [string]$Status, [int]$ExitCode = -1)

    [ordered]@{
        timestamp = (Get-Date).ToUniversalTime().ToString('o')
        stage = $Stage
        status = $Status
        exitCode = $ExitCode
        maxParallelPairs = $MaxParallelPairs
    } | ConvertTo-Json -Compress | Add-Content -LiteralPath $controlLog -Encoding utf8
}

function Test-ValidPair {
    param(
        [string]$Task,
        [string]$BeforeRoot,
        [string]$AfterRoot
    )

    foreach ($entry in @(
        @{ snapshot = 'before'; root = $BeforeRoot },
        @{ snapshot = 'after'; root = $AfterRoot }
    )) {
        $root = Join-Path $PlanRoot $entry.root
        if (-not (Test-Path -LiteralPath $root)) {
            return $false
        }
        $trialResults = Get-ChildItem -LiteralPath $root -Recurse -File -Filter 'result.json' -ErrorAction SilentlyContinue |
            Where-Object { $_.Directory.Parent.Name -match '^\d{4}-\d{2}-\d{2}__' } |
            Sort-Object LastWriteTime -Descending
        $match = $null
        foreach ($result in $trialResults) {
            try {
                $payload = Get-Content -LiteralPath $result.FullName -Raw | ConvertFrom-Json
            } catch {
                continue
            }
            if ($payload.task_name -eq "terminal-bench/$Task") {
                $match = $payload
                break
            }
        }
        if ($null -eq $match -or $null -eq $match.finished_at) {
            return $false
        }
        $turnIssue = [string]$match.agent_result.metadata.turnError
        if (
            $turnIssue -match 'provider request failed' -or
            $turnIssue -match 'provider stream returned an error' -or
            $turnIssue -match 'error sending request' -or
            $turnIssue -match 'error decoding response body' -or
            $turnIssue -match 'upstream_error'
        ) {
            return $false
        }
    }
    return $true
}

function Invoke-RecoveryPair {
    param(
        [string]$Task,
        [ValidateSet('before', 'after')]
        [string]$First,
        [string]$BeforeRoot,
        [string]$AfterRoot,
        [string]$RunLogName
    )

    $arguments = @{
        PlanRoot = $PlanRoot
        BeforeServerBinary = $BeforeServerBinary
        AfterServerBinary = $AfterServerBinary
        EnvFile = $EnvFile
        BeforeJobsDirectoryName = $BeforeRoot
        AfterJobsDirectoryName = $AfterRoot
        RunLogFileName = $RunLogName
        OnlyPairTask = $Task
        OnlyPairFirst = $First
    }
    & $pairRunner @arguments
}

Write-ControlEvent -Stage 'inflight_pairs' -Status 'waiting'
Wait-Process -Id $InitialControllerPid -ErrorAction SilentlyContinue
Write-ControlEvent -Stage 'inflight_pairs' -Status 'finished' -ExitCode 0
Write-ControlEvent -Stage 'cooldown' -Status 'started'
Start-Sleep -Seconds $CooldownSeconds
Write-ControlEvent -Stage 'cooldown' -Status 'finished' -ExitCode 0

Write-ControlEvent -Stage 'rs_archive_clone' -Status 'started'
try {
    Invoke-RecoveryPair -Task 'rs-archive-clone' -First 'before' -BeforeRoot $rsBeforeRoot -AfterRoot $rsAfterRoot -RunLogName 'terminal-bench-rs-overload-recovery-20260827.run-log.jsonl'
} catch {
    Write-ControlEvent -Stage 'rs_archive_clone' -Status 'finished' -ExitCode 1
    throw
}
if (-not (Test-ValidPair -Task 'rs-archive-clone' -BeforeRoot $rsBeforeRoot -AfterRoot $rsAfterRoot)) {
    Write-ControlEvent -Stage 'rs_archive_clone' -Status 'finished' -ExitCode 1
    throw 'RS archive recovery pair was not valid; remaining benchmarks were not started.'
}
Write-ControlEvent -Stage 'rs_archive_clone' -Status 'finished' -ExitCode 0

if (-not (Test-ValidPair -Task 'distributed-dedup' -BeforeRoot $currentBeforeRoot -AfterRoot $currentAfterRoot)) {
    Write-ControlEvent -Stage 'distributed_dedup' -Status 'started'
    try {
        Invoke-RecoveryPair -Task 'distributed-dedup' -First 'after' -BeforeRoot $distributedBeforeRoot -AfterRoot $distributedAfterRoot -RunLogName 'terminal-bench-distributed-overload-recovery-20260827.run-log.jsonl'
    } catch {
        Write-ControlEvent -Stage 'distributed_dedup' -Status 'finished' -ExitCode 1
        throw
    }
    if (-not (Test-ValidPair -Task 'distributed-dedup' -BeforeRoot $distributedBeforeRoot -AfterRoot $distributedAfterRoot)) {
        Write-ControlEvent -Stage 'distributed_dedup' -Status 'finished' -ExitCode 1
        throw 'Distributed dedup recovery pair was not valid; remaining benchmarks were not started.'
    }
    Write-ControlEvent -Stage 'distributed_dedup' -Status 'finished' -ExitCode 0
}

$continuationArguments = @{
    PilotRunnerPid = 0
    PlanRoot = $PlanRoot
    BeforeServerBinary = $BeforeServerBinary
    AfterServerBinary = $AfterServerBinary
    EnvFile = $EnvFile
    MaxParallelPairs = $MaxParallelPairs
    StartRemainingTerminalTask = 'bun-sourcemap-leak'
    TerminalBeforeJobsDirectoryName = $remainingBeforeRoot
    TerminalAfterJobsDirectoryName = $remainingAfterRoot
    TerminalBeforeSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-pilot-mp-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-upstream-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-quota-restored-ontology-20260827',
        $currentBeforeRoot,
        $rsBeforeRoot,
        $distributedBeforeRoot,
        $remainingBeforeRoot
    )
    TerminalAfterSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-pilot-mp-20260827-clean',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-upstream-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-quota-restored-ontology-20260827',
        $currentAfterRoot,
        $rsAfterRoot,
        $distributedAfterRoot,
        $remainingAfterRoot
    )
}
Write-ControlEvent -Stage 'remaining_benchmarks' -Status 'started'
try {
    & $continuation @continuationArguments
    Write-ControlEvent -Stage 'remaining_benchmarks' -Status 'finished' -ExitCode 0
} catch {
    Write-ControlEvent -Stage 'remaining_benchmarks' -Status 'finished' -ExitCode 1
    throw
}
