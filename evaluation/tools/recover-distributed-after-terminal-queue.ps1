param(
    [Parameter(Mandatory = $true)]
    [int]$TerminalQueuePid,

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
$pairRunner = Join-Path $PSScriptRoot 'run-terminal-bench-paired.ps1'
$beforeRoot = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-recovery-overload-distributed-retry2-20260827'
$afterRoot = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-recovery-overload-distributed-retry2-20260827'
$completionPath = Join-Path $PlanRoot 'distributed-retry2-completion-20260827.json'

foreach ($path in @($pairRunner, $BeforeServerBinary, $AfterServerBinary, $EnvFile)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required distributed recovery input is missing: $path"
    }
}

$terminalQueue = Get-Process -Id $TerminalQueuePid -ErrorAction SilentlyContinue
if ($null -ne $terminalQueue) {
    $terminalQueue.WaitForExit()
}

# A missing snapshot or provider rejection is retried, while a valid side is
# retained. Three attempts cover transient endpoint overload without masking a
# persistent infrastructure failure as an agent result.
$recovered = $false
for ($attempt = 0; $attempt -lt 3 -and -not $recovered; $attempt++) {
    try {
        & $pairRunner @{
            PlanRoot = $PlanRoot
            BeforeServerBinary = $BeforeServerBinary
            AfterServerBinary = $AfterServerBinary
            EnvFile = $EnvFile
            BeforeJobsDirectoryName = $beforeRoot
            AfterJobsDirectoryName = $afterRoot
            RunLogFileName = 'terminal-bench-distributed-retry2-20260827.run-log.jsonl'
            OnlyPairTask = 'distributed-dedup'
            OnlyPairFirst = 'after'
            RetryProviderFailures = $true
        }
        $recovered = $true
    } catch {
        if ($attempt -eq 2) {
            throw
        }
    }
}

[ordered]@{
    schemaVersion = 1
    task = 'distributed-dedup'
    beforeRoot = $beforeRoot
    afterRoot = $afterRoot
    completedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
} | ConvertTo-Json | Set-Content -LiteralPath $completionPath -Encoding utf8
