param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [string]$JobsDirectoryName = 'terminal-bench-codex-account-default-20260828',

    [ValidateRange(1, 2)]
    [int]$MaxParallelTasks = 2,

    [ValidateRange(180, 3600)]
    [int]$RunTimeoutSeconds = 1800,

    [ValidateRange(240, 4800)]
    [int]$HarnessTimeoutSeconds = 2100,

    [string[]]$ExcludeTask = @(),

    # Use a JSON array when a controller needs to resume a queue with more
    # than one exclusion.  `pwsh -File` binds only the first space-separated
    # value to a string-array parameter; subsequent values can otherwise be
    # mistaken for positional parameters such as JobsDirectoryName.
    [string]$TaskListPath = '',

    [string]$OnlyTask = ''
)

$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$env:PYTHONPATH = $sourceRoot
$harbor = 'C:\Users\Stargo\AppData\Roaming\uv\tools\harbor\Scripts\harbor.exe'
if (-not (Test-Path -LiteralPath $harbor)) {
    throw "Harbor executable is missing: $harbor"
}

$jobsRoot = Join-Path $PlanRoot $JobsDirectoryName
New-Item -ItemType Directory -Force -Path $jobsRoot | Out-Null

# This is exactly the previously valid Terminal-Bench set.  Coq, photonic,
# and protein remain excluded because they lacked a valid paired result in the
# before/after study; this Codex baseline is intentionally comparable.
$tasks = @(
    'bun-sourcemap-leak',
    'cargo-flight-dispatch',
    'data-anonymization',
    'distributed-dedup',
    'freecad-spring-clip',
    'hof-topology-interpenetration',
    'mp-checkpoint-consolidation',
    'ontology-kg-querying',
    'retro-console-soc',
    'roy-polymorph-cn',
    'rs-archive-clone',
    'shadow-relay',
    'telecom-entity-resolution',
    'vpp-loss-divergence',
    'wdm-design'
)
if ($TaskListPath) {
    if ($OnlyTask -or $ExcludeTask.Count -gt 0) {
        throw 'TaskListPath cannot be combined with OnlyTask or ExcludeTask.'
    }
    $resolvedTaskListPath = (Resolve-Path -LiteralPath $TaskListPath).Path
    $requestedTasks = @(Get-Content -LiteralPath $resolvedTaskListPath -Raw | ConvertFrom-Json)
    if ($requestedTasks.Count -eq 0) {
        throw 'TaskListPath did not contain any tasks.'
    }
    if (@($requestedTasks | Select-Object -Unique).Count -ne $requestedTasks.Count) {
        throw 'TaskListPath contains duplicate task names.'
    }
    foreach ($task in $requestedTasks) {
        if ($tasks -notcontains $task) {
            throw "TaskListPath contains a task outside the selected valid Terminal-Bench set: $task"
        }
    }
    $tasks = @($requestedTasks)
} elseif ($OnlyTask) {
    if ($tasks -notcontains $OnlyTask) {
        throw "OnlyTask is not in the selected valid Terminal-Bench set: $OnlyTask"
    }
    $tasks = @($OnlyTask)
} elseif ($ExcludeTask.Count -gt 0) {
    foreach ($task in $ExcludeTask) {
        if ($tasks -notcontains $task) {
            throw "ExcludeTask is not in the selected valid Terminal-Bench set: $task"
        }
    }
    $tasks = @($tasks | Where-Object { $_ -notin $ExcludeTask })
    if ($tasks.Count -eq 0) {
        throw 'ExcludeTask removed every selected Terminal-Bench task.'
    }
}

function Stop-ChildProcessTree {
    param([Parameter(Mandatory = $true)][int]$RootProcessId)

    $processes = Get-CimInstance Win32_Process
    $frontier = [System.Collections.Generic.Queue[int]]::new()
    $frontier.Enqueue($RootProcessId)
    $seen = [System.Collections.Generic.HashSet[int]]::new()
    $descendants = [System.Collections.Generic.List[int]]::new()
    while ($frontier.Count -gt 0) {
        $current = $frontier.Dequeue()
        if (-not $seen.Add($current)) { continue }
        foreach ($child in @($processes | Where-Object { $_.ParentProcessId -eq $current })) {
            $descendants.Add([int]$child.ProcessId)
            $frontier.Enqueue([int]$child.ProcessId)
        }
    }
    foreach ($childId in @($descendants | Sort-Object -Descending)) {
        Stop-Process -Id $childId -Force -ErrorAction SilentlyContinue
    }
    Stop-Process -Id $RootProcessId -Force -ErrorAction SilentlyContinue
}

function Start-CodexTerminalTask {
    param([Parameter(Mandatory = $true)][string]$Task)

    $taskJobs = Join-Path $jobsRoot $Task
    New-Item -ItemType Directory -Force -Path $taskJobs | Out-Null
    $stdout = Join-Path $taskJobs 'launcher.stdout.log'
    $stderr = Join-Path $taskJobs 'launcher.stderr.log'
    $arguments = @(
        'run',
        '--dataset', 'terminal-bench/terminal-bench@latest',
        '--agent', 'evaluation.integrations.harbor.codex_container_agent:CodexContainerAgent',
        '--agent-kwarg', "run_timeout_sec=$RunTimeoutSeconds",
        '--include-task-name', "terminal-bench/$Task",
        '--n-attempts', '1',
        '--n-concurrent', '1',
        '--env', 'docker',
        '--jobs-dir', $taskJobs,
        '--yes'
    )
    $process = Start-Process -FilePath $harbor -ArgumentList $arguments `
        -WorkingDirectory $sourceRoot -WindowStyle Hidden `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    return [pscustomobject]@{
        task = $Task
        process = $process
        jobs = $taskJobs
        startedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
    }
}

$pending = [System.Collections.ArrayList]::new()
foreach ($task in $tasks) { [void]$pending.Add($task) }
$active = [System.Collections.ArrayList]::new()
$completed = [System.Collections.ArrayList]::new()

while ($pending.Count -gt 0 -or $active.Count -gt 0) {
    while ($pending.Count -gt 0 -and $active.Count -lt $MaxParallelTasks) {
        $task = $pending[0]
        $pending.RemoveAt(0)
        [void]$active.Add((Start-CodexTerminalTask -Task $task))
    }
    Start-Sleep -Seconds 2
    foreach ($entry in @($active)) {
        $entry.process.Refresh()
        $elapsed = ((Get-Date).ToUniversalTime() - [datetime]$entry.startedAtUtc).TotalSeconds
        if (-not $entry.process.HasExited -and $elapsed -ge $HarnessTimeoutSeconds) {
            Stop-ChildProcessTree -RootProcessId $entry.process.Id
        }
        $entry.process.Refresh()
        if (-not $entry.process.HasExited) { continue }
        [void]$completed.Add([ordered]@{
            task = $entry.task
            exitCode = $entry.process.ExitCode
            startedAtUtc = $entry.startedAtUtc
            finishedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
            jobs = $entry.jobs
        })
        [void]$active.Remove($entry)
    }
}

[ordered]@{
    schemaVersion = 1
    modelSelection = 'codex-account-default-no-model-flag'
    maxParallelTasks = $MaxParallelTasks
    runTimeoutSeconds = $RunTimeoutSeconds
    completed = @($completed)
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $jobsRoot 'manifest.json') -Encoding utf8

if (@($completed | Where-Object { $_.exitCode -ne 0 }).Count -gt 0) {
    throw 'One or more Codex Terminal-Bench task launchers failed. Check per-task logs before retrying.'
}
