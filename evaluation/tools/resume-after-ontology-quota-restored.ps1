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
    [int]$OntologyRunnerPid,

    [ValidateRange(1, 4)]
    [int]$MaxParallelPairs = 2
)

$ErrorActionPreference = 'Stop'

$continuation = Join-Path $PSScriptRoot 'continue-public-benchmarks-after-pilot.ps1'
$ontologyRunLog = Join-Path $PlanRoot 'terminal-bench-ontology-quota-restored-20260827.run-log.jsonl'
$controlLog = Join-Path $PlanRoot 'resume-after-ontology-quota-restored-20260827.jsonl'
$ontologyBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-quota-restored-ontology-20260827'
$ontologyAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-quota-restored-ontology-20260827'
$remainingBeforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-quota-restored-par2-20260827'
$remainingAfterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-quota-restored-par2-20260827'

foreach ($path in @($continuation, $BeforeServerBinary, $AfterServerBinary, $EnvFile)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required resume input is missing: $path"
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

function Test-ValidOntologyPair {
    if (-not (Test-Path -LiteralPath $ontologyRunLog)) {
        return $false
    }

    $events = @()
    foreach ($line in Get-Content -LiteralPath $ontologyRunLog) {
        try {
            $events += $line | ConvertFrom-Json
        } catch {
            return $false
        }
    }

    foreach ($snapshot in @('before', 'after')) {
        $finished = @($events | Where-Object {
            $_.snapshot -eq $snapshot -and $_.status -eq 'finished' -and $_.exitCode -eq 0
        })
        $providerFailures = @($events | Where-Object {
            $_.snapshot -eq $snapshot -and $_.status -eq 'provider_failure'
        })
        if ($finished.Count -eq 0 -or $providerFailures.Count -gt 0) {
            return $false
        }
    }
    return $true
}

Write-ControlEvent -Stage 'ontology_pair' -Status 'waiting'
Wait-Process -Id $OntologyRunnerPid -ErrorAction SilentlyContinue
if (-not (Test-ValidOntologyPair)) {
    Write-ControlEvent -Stage 'ontology_pair' -Status 'failed_validation' -ExitCode 1
    throw 'Ontology recovery pair was not valid; remaining benchmarks were not started.'
}
Write-ControlEvent -Stage 'ontology_pair' -Status 'validated' -ExitCode 0

$continuationArguments = @{
    PilotRunnerPid = 0
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

Write-ControlEvent -Stage 'remaining_benchmarks' -Status 'started'
try {
    & $continuation @continuationArguments
    Write-ControlEvent -Stage 'remaining_benchmarks' -Status 'finished' -ExitCode 0
} catch {
    Write-ControlEvent -Stage 'remaining_benchmarks' -Status 'finished' -ExitCode 1
    throw
}
