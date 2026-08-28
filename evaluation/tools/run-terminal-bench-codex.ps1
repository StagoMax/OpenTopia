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

    # In pwsh -File mode, only the first whitespace-separated value binds to
    # a string-array parameter. Controllers should use this JSON list when
    # resuming more than one selected task.
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
$allTasks = @(
    'bun-sourcemap-leak', 'cargo-flight-dispatch', 'data-anonymization',
    'distributed-dedup', 'freecad-spring-clip', 'hof-topology-interpenetration',
    'mp-checkpoint-consolidation', 'ontology-kg-querying', 'retro-console-soc',
    'roy-polymorph-cn', 'rs-archive-clone', 'shadow-relay',
    'telecom-entity-resolution', 'vpp-loss-divergence', 'wdm-design'
)

if ($TaskListPath) {
    if ($OnlyTask -or $ExcludeTask.Count -gt 0) {
        throw 'TaskListPath cannot be combined with OnlyTask or ExcludeTask.'
    }
    $tasks = @(Get-Content -LiteralPath (Resolve-Path -LiteralPath $TaskListPath).Path -Raw | ConvertFrom-Json)
    if ($tasks.Count -eq 0 -or @($tasks | Select-Object -Unique).Count -ne $tasks.Count) {
        throw 'TaskListPath must contain a nonempty, unique task list.'
    }
    foreach ($task in $tasks) {
        if ($allTasks -notcontains $task) { throw "TaskListPath contains an unsupported task: $task" }
    }
} elseif ($OnlyTask) {
    if ($allTasks -notcontains $OnlyTask) { throw "OnlyTask is not selected: $OnlyTask" }
    $tasks = @($OnlyTask)
} else {
    foreach ($task in $ExcludeTask) {
        if ($allTasks -notcontains $task) { throw "ExcludeTask is not selected: $task" }
    }
    $tasks = @($allTasks | Where-Object { $_ -notin $ExcludeTask })
}

function Stop-ChildProcessTree {
    param([Parameter(Mandatory = $true)][int]$RootProcessId)
    $processes = Get-CimInstance Win32_Process
    $queue = [System.Collections.Generic.Queue[int]]::new()
    $seen = [System.Collections.Generic.HashSet[int]]::new()
    $queue.Enqueue($RootProcessId)
    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        if (-not $seen.Add($current)) { continue }
        foreach ($child in @($processes | Where-Object { $_.ParentProcessId -eq $current })) {
            $queue.Enqueue([int]$child.ProcessId)
        }
    }
    foreach ($processId in @($seen | Sort-Object -Descending)) {
        Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue
    }
}

function Start-CodexTerminalTask {
    param([Parameter(Mandatory = $true)][string]$Task)

    $taskJobs = Join-Path $jobsRoot $Task
    New-Item -ItemType Directory -Force -Path $taskJobs | Out-Null
    $stdout = Join-Path $taskJobs 'launcher.stdout.log'
    $stderr = Join-Path $taskJobs 'launcher.stderr.log'
    # This Windows host can fail to create redirect paths inside Start-Process.
    # Pre-creating them makes such a failure unambiguously infrastructure-only.
    New-Item -ItemType File -Force -Path $stdout | Out-Null
    New-Item -ItemType File -Force -Path $stderr | Out-Null
    $arguments = @(
        'run', '--dataset', 'terminal-bench/terminal-bench@latest',
        '--agent', 'evaluation.integrations.harbor.codex_container_agent:CodexContainerAgent',
        '--agent-kwarg', "run_timeout_sec=$RunTimeoutSeconds",
        '--include-task-name', "terminal-bench/$Task",
        '--n-attempts', '1', '--n-concurrent', '1', '--env', 'docker',
        '--jobs-dir', $taskJobs, '--yes'
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
    throw 'One or more Codex Terminal-Bench task launchers failed.'
}
