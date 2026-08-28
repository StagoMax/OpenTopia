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
$controlLog = Join-Path $PlanRoot 'terminal-bench-ontology-overload-recovery-20260827.jsonl'
$ontologyBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-overload-ontology-20260827'
$ontologyAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-overload-ontology-20260827'
$remainingBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-par2-20260827'
$remainingAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-par2-20260827'

foreach ($path in @($pairRunner, $continuation, $BeforeServerBinary, $AfterServerBinary, $EnvFile)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required overload-recovery input is missing: $path"
    }
}

function Write-RecoveryEvent {
    param([string]$Stage, [int]$ExitCode = -1)

    [ordered]@{
        timestamp = (Get-Date).ToUniversalTime().ToString('o')
        stage = $Stage
        task = 'ontology-kg-querying'
        exitCode = $ExitCode
        maxParallelPairs = $MaxParallelPairs
    } | ConvertTo-Json -Compress | Add-Content -LiteralPath $controlLog -Encoding utf8
}

$pairArguments = @{
    PlanRoot = $PlanRoot
    BeforeServerBinary = $BeforeServerBinary
    AfterServerBinary = $AfterServerBinary
    EnvFile = $EnvFile
    BeforeJobsDirectoryName = $ontologyBeforeRoot
    AfterJobsDirectoryName = $ontologyAfterRoot
    RunLogFileName = 'terminal-bench-ontology-overload-recovery-20260827.run-log.jsonl'
    OnlyPairTask = 'ontology-kg-querying'
    OnlyPairFirst = 'before'
}
Write-RecoveryEvent -Stage 'pair_started'
try {
    & $pairRunner @pairArguments
    Write-RecoveryEvent -Stage 'pair_finished' -ExitCode 0
} catch {
    Write-RecoveryEvent -Stage 'pair_finished' -ExitCode 1
    throw
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
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-upstream-20260827',
        $ontologyBeforeRoot,
        $remainingBeforeRoot
    )
    TerminalAfterSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-pilot-mp-20260827-clean',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-upstream-20260827',
        $ontologyAfterRoot,
        $remainingAfterRoot
    )
}
Write-RecoveryEvent -Stage 'resume_remaining_terminal'
& $continuation @continuationArguments
