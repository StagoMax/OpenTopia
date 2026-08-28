param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [int]$TerminalSchedulerPid,

    [Parameter(Mandatory = $true)]
    [int]$SweSchedulerPid,

    [Parameter(Mandatory = $true)]
    [int]$TerminalWorkerPid,

    [Parameter(Mandatory = $true)]
    [int]$SweWorkerPid
)

$ErrorActionPreference = 'Stop'
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$terminalRoot = Join-Path $PlanRoot 'terminal-bench-codex-account-default-20260828'
$sweRoot = Join-Path $PlanRoot 'swebench-codex-account-default-20260828'
$terminalTask = 'terminal-bench/bun-sourcemap-leak'
$sweInstance = 'pallets__flask-5014'

function Stop-SchedulerOnly {
    param([int]$ProcessId)
    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($process) {
        # The current single-task worker is a separate child process.  Do not
        # terminate its tree: let it produce an official result, then restart
        # the remaining queues with four total Codex slots.
        Stop-Process -Id $ProcessId -Force
    }
}

function Test-TerminalValid {
    param([string]$TaskName)
    $results = Get-ChildItem -LiteralPath $terminalRoot -Recurse -File -Filter 'result.json' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending
    foreach ($path in $results) {
        try { $result = Get-Content -LiteralPath $path.FullName -Raw | ConvertFrom-Json } catch { continue }
        if ($result.task_name -ne $TaskName) { continue }
        if ($result.exception_info -ne $null) { continue }
        $reward = $result.verifier_result.rewards.reward
        $commands = $result.agent_result.metadata.commandExecutionsFinished
        if ($null -ne $reward -and $null -ne $commands) {
            try { return [double]$commands -gt 0 } catch { return $false }
        }
    }
    return $false
}

function Test-SweValid {
    param([string]$InstanceId)
    $scores = Get-ChildItem -LiteralPath $sweRoot -Recurse -File -Filter 'official-score-lf-fixed.json' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending
    foreach ($path in $scores) {
        try { $score = Get-Content -LiteralPath $path.FullName -Raw | ConvertFrom-Json } catch { continue }
        $observation = $score.result[1].$InstanceId
        if ($null -ne $observation) { return $true }
    }
    return $false
}

Stop-SchedulerOnly -ProcessId $TerminalSchedulerPid
Stop-SchedulerOnly -ProcessId $SweSchedulerPid
foreach ($worker in @($TerminalWorkerPid, $SweWorkerPid)) {
    Wait-Process -Id $worker -ErrorAction SilentlyContinue
}

$terminalExcludes = @('mp-checkpoint-consolidation')
if (Test-TerminalValid -TaskName $terminalTask) { $terminalExcludes += 'bun-sourcemap-leak' }
$sweExcludes = @()
if (Test-SweValid -InstanceId $sweInstance) { $sweExcludes += $sweInstance }

$terminalRunner = Join-Path $PSScriptRoot 'run-terminal-bench-codex.ps1'
# Pass the selected tasks through a single file argument.  When pwsh is
# launched with -File, a string-array parameter only consumes its first
# whitespace-separated value; using -ExcludeTask a b could bind b to an
# unrelated positional parameter and redirect result files.
$allTerminalTasks = @(
    'bun-sourcemap-leak', 'cargo-flight-dispatch', 'data-anonymization',
    'distributed-dedup', 'freecad-spring-clip', 'hof-topology-interpenetration',
    'mp-checkpoint-consolidation', 'ontology-kg-querying', 'retro-console-soc',
    'roy-polymorph-cn', 'rs-archive-clone', 'shadow-relay',
    'telecom-entity-resolution', 'vpp-loss-divergence', 'wdm-design'
)
$terminalTaskList = @($allTerminalTasks | Where-Object { $_ -notin $terminalExcludes })
$terminalTaskListPath = Join-Path $PlanRoot 'codex-terminal-remaining-20260828.json'
$terminalTaskList | ConvertTo-Json | Set-Content -LiteralPath $terminalTaskListPath -Encoding utf8
$terminalArgs = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$terminalRunner,'-PlanRoot',$PlanRoot,'-MaxParallelTasks','2','-TaskListPath',$terminalTaskListPath)
$terminal = Start-Process -FilePath 'pwsh.exe' -ArgumentList $terminalArgs -WorkingDirectory $sourceRoot -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $PlanRoot 'codex-terminal-par4-20260828.stdout.log') `
    -RedirectStandardError (Join-Path $PlanRoot 'codex-terminal-par4-20260828.stderr.log') -PassThru

$sweRunner = Join-Path $PSScriptRoot 'run-swebench-codex.ps1'
$sweArgs = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$sweRunner,'-PlanRoot',$PlanRoot,'-MaxParallelTasks','2')
if ($sweExcludes.Count -gt 0) { $sweArgs += '-ExcludeInstance'; $sweArgs += $sweExcludes }
$swe = Start-Process -FilePath 'pwsh.exe' -ArgumentList $sweArgs -WorkingDirectory $sourceRoot -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $PlanRoot 'codex-swebench-par4-20260828.stdout.log') `
    -RedirectStandardError (Join-Path $PlanRoot 'codex-swebench-par4-20260828.stderr.log') -PassThru

[pscustomobject]@{
    switchedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
    terminalWorkerWasValid = $terminalExcludes -contains 'bun-sourcemap-leak'
    sweWorkerWasValid = $sweExcludes -contains $sweInstance
    terminalSchedulerPid = $terminal.Id
    sweSchedulerPid = $swe.Id
    totalTargetConcurrency = 4
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $PlanRoot 'codex-parallelism-switch-20260828.json') -Encoding utf8
