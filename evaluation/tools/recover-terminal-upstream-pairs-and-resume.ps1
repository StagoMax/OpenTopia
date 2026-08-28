param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [string]$BeforeServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$AfterServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$EnvFile,

    [ValidateRange(1, 4)]
    [int]$MaxParallelPairs = 2
)

$ErrorActionPreference = 'Stop'
$pairRunner = Join-Path $PSScriptRoot 'run-terminal-bench-paired.ps1'
$continuation = Join-Path $PSScriptRoot 'continue-public-benchmarks-after-pilot.ps1'
$controlLog = Join-Path $PlanRoot 'terminal-bench-upstream-recovery-20260827.jsonl'
$recoveryBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-upstream-20260827'
$recoveryAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-upstream-20260827'
$remainingBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-par2-20260827'
$remainingAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-par2-20260827'

foreach ($path in @($pairRunner, $continuation, $BeforeServerBinary, $AfterServerBinary, $EnvFile)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required recovery input is missing: $path"
    }
}

function Write-RecoveryEvent {
    param([string]$Stage, [string]$Task = '', [int]$ExitCode = -1)

    [ordered]@{
        timestamp = (Get-Date).ToUniversalTime().ToString('o')
        stage = $Stage
        task = $Task
        exitCode = $ExitCode
        maxParallelPairs = $MaxParallelPairs
    } | ConvertTo-Json -Compress | Add-Content -LiteralPath $controlLog -Encoding utf8
}

# Both original before snapshots had the same content-free upstream_error hash.
# Rerun complete pairs (rather than only before) to retain a clean, local
# before/after pairing and avoid mixing timestamps across the recovery.
$pairs = @('ontology-kg-querying', 'telecom-entity-resolution')
$active = [System.Collections.ArrayList]::new()
foreach ($task in $pairs) {
    $taskSlug = $task -replace '[^A-Za-z0-9._-]', '-'
    $beforeJobs = Join-Path $recoveryBeforeRoot $taskSlug
    $afterJobs = Join-Path $recoveryAfterRoot $taskSlug
    $stdout = Join-Path $PlanRoot "terminal-bench-recovery-$taskSlug.stdout.log"
    $stderr = Join-Path $PlanRoot "terminal-bench-recovery-$taskSlug.stderr.log"
    $runLog = "terminal-bench-recovery-$taskSlug.run-log.jsonl"
    $arguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $pairRunner,
        '-PlanRoot', $PlanRoot,
        '-BeforeServerBinary', $BeforeServerBinary,
        '-AfterServerBinary', $AfterServerBinary,
        '-EnvFile', $EnvFile,
        '-BeforeJobsDirectoryName', $beforeJobs,
        '-AfterJobsDirectoryName', $afterJobs,
        '-RunLogFileName', $runLog,
        '-OnlyPairTask', $task,
        '-OnlyPairFirst', 'before'
    )
    Write-RecoveryEvent -Stage 'pair_started' -Task $task
    $process = Start-Process -FilePath 'pwsh.exe' -ArgumentList $arguments -WorkingDirectory (Get-Location).Path -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    [void]$active.Add([pscustomobject]@{ task = $task; process = $process })
}

$completed = [System.Collections.ArrayList]::new()
foreach ($entry in $active) {
    $entry.process.WaitForExit()
    [void]$completed.Add([ordered]@{
        task = $entry.task
        exitCode = $entry.process.ExitCode
        finishedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
    })
    Write-RecoveryEvent -Stage 'pair_finished' -Task $entry.task -ExitCode $entry.process.ExitCode
}
$manifestPath = Join-Path $PlanRoot 'terminal-bench-upstream-recovery-manifest.json'
@{
    schemaVersion = 1
    completed = @($completed)
    recoveryBeforeRoot = $recoveryBeforeRoot
    recoveryAfterRoot = $recoveryAfterRoot
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8

if (@($completed | Where-Object { $_.exitCode -ne 0 }).Count -gt 0) {
    throw 'An upstream-recovery pair failed; remaining benchmark work was not started.'
}

$continuationArguments = @{
    PilotRunnerPid = 45516
    PlanRoot = $PlanRoot
    BeforeServerBinary = $BeforeServerBinary
    AfterServerBinary = $AfterServerBinary
    EnvFile = $EnvFile
    MaxParallelPairs = $MaxParallelPairs
    StartRemainingTerminalTask = 'rs-archive-clone'
    TerminalBeforeJobsDirectoryName = $remainingBeforeRoot
    TerminalAfterJobsDirectoryName = $remainingAfterRoot
    TerminalBeforeSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-pilot-mp-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827',
        $recoveryBeforeRoot,
        $remainingBeforeRoot
    )
    TerminalAfterSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-pilot-mp-20260827-clean',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827',
        $recoveryAfterRoot,
        $remainingAfterRoot
    )
}
Write-RecoveryEvent -Stage 'resume_remaining_terminal'
& $continuation @continuationArguments
