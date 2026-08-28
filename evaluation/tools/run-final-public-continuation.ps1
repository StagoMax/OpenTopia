param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [string]$BeforeServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$AfterServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$EnvFile
)

$ErrorActionPreference = 'Stop'
$continuation = Join-Path $PSScriptRoot 'continue-public-benchmarks-after-pilot.ps1'

$continuationArguments = @{
    PilotRunnerPid = 0
    PlanRoot = $PlanRoot
    BeforeServerBinary = $BeforeServerBinary
    AfterServerBinary = $AfterServerBinary
    EnvFile = $EnvFile
    MaxParallelPairs = 4
    StartRemainingTerminalTask = 'bun-sourcemap-leak'
    TerminalBeforeJobsDirectoryName = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-after-overload-par2-20260827'
    TerminalAfterJobsDirectoryName = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-after-overload-par2-20260827'
    TerminalBeforeSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-pilot-mp-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-upstream-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-quota-restored-ontology-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-quota-restored-par2-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-overload-rs-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-overload-distributed-retry2-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-remaining-after-overload-par2-20260827'
    )
    TerminalAfterSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-pilot-mp-20260827-clean',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-upstream-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-quota-restored-ontology-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-quota-restored-par2-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-overload-rs-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-overload-distributed-retry2-20260827',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-remaining-after-overload-par2-20260827'
    )
}
& $continuation @continuationArguments
