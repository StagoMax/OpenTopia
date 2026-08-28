param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [string]$BeforeServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$AfterServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$EnvFile,

    [string]$BeforeJobsDirectoryName = 'terminal-bench-unbounded-before-6d52',

    [string]$AfterJobsDirectoryName = 'terminal-bench-unbounded-after-current',

    [string]$RunLogFileName = 'terminal-bench-unbounded-current-run-log.jsonl',

    [string]$ResumePairTask = '',

    [string]$ResumeAfterTask = '',

    # Run precisely one before/after pair and then exit.  This is used for an
    # isolated recovery task (such as coq) after its no-model Docker preflight.
    [string]$OnlyPairTask = '',

    # Optional first snapshot for an isolated pair.  The parallel dispatcher
    # uses the fixed alternating order below to limit endpoint-time drift.
    [ValidateSet('', 'before', 'after')]
    [string]$OnlyPairFirst = '',

    # Retry exactly one side of a pair after a provider/transport failure.
    # The successful counterpart remains reusable by the paired summarizer.
    [ValidateSet('', 'before', 'after')]
    [string]$OnlySnapshot = '',

    # In recovery mode, preserve terminal snapshots that already have a
    # usable result and execute only missing/provider-failed snapshots.
    [switch]$RetryProviderFailures,

    # Start a fresh recovery run at this task in the fixed pair order.  This
    # avoids re-running already valid pairs after an infrastructure-only fault.
    [string]$StartRemainingTask = '',

    [int]$WaitForPid = 0,

    # The agent turn itself is controlled at 30 minutes.  Harbor can then
    # spend additional time collecting events and running the verifier; this
    # outer watchdog prevents an unavailable verifier from stalling every
    # later pair indefinitely.
    [ValidateRange(1801, 7200)]
    [int]$HarnessTimeoutSeconds = 2700
)

$ErrorActionPreference = 'Stop'

# The outer Terminal-Bench Docker container is the safety boundary.  Both
# snapshots therefore use OpenTopia's common full-access mode inside it.
$env:PYTHONUTF8 = '1'
$env:PYTHONPATH = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

$harbor = 'C:\Users\Stargo\AppData\Roaming\uv\tools\harbor\Scripts\harbor.exe'
$beforeJobs = Join-Path $PlanRoot $BeforeJobsDirectoryName
$afterJobs = Join-Path $PlanRoot $AfterJobsDirectoryName
$runLog = Join-Path $PlanRoot $RunLogFileName

foreach ($directory in @($beforeJobs, $afterJobs)) {
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
}

function Write-RunEvent {
    param(
        [string]$Task,
        [string]$Snapshot,
        [string]$Status,
        [int]$ExitCode = -1
    )

    [pscustomobject]@{
        timestamp = (Get-Date).ToUniversalTime().ToString('o')
        task = $Task
        snapshot = $Snapshot
        status = $Status
        exitCode = $ExitCode
        rolloutLimitTokens = $null
        model = 'gpt-5.6-terra'
        reasoningEffort = 'high'
        maxOutputTokens = 8192
        taskTimeoutSeconds = 1800
    } | ConvertTo-Json -Compress | Add-Content -LiteralPath $runLog -Encoding utf8
}

function Get-ProviderFailure {
    param(
        [string]$Task,
        [string]$JobsDir
    )

    # Harbor records a provider rejection as a completed trial, rather than a
    # launcher failure.  Read only the terminal error metadata; benchmark
    # instructions, model text, and tool payloads are intentionally ignored.
    $expectedTaskName = "terminal-bench/$Task"
    $results = Get-ChildItem -LiteralPath $JobsDir -Recurse -File -Filter 'result.json' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending
    foreach ($result in $results) {
        try {
            $payload = Get-Content -LiteralPath $result.FullName -Raw | ConvertFrom-Json
        } catch {
            continue
        }
        if ($payload.task_name -ne $expectedTaskName) {
            continue
        }
        $turnError = $payload.agent_result.metadata.turnError
        if ($turnError -isnot [string]) {
            return $null
        }
        $normalized = $turnError.ToLowerInvariant()
        if (
            $normalized.Contains('provider request failed') -or
            $normalized.Contains('provider stream returned an error') -or
            $normalized.Contains('error sending request') -or
            $normalized.Contains('error decoding response body') -or
            $normalized.Contains('upstream_error')
        ) {
            return $turnError
        }
        return $null
    }
    return 'Harbor finished without a readable task result.'
}

function Stop-ChildProcessTree {
    param([Parameter(Mandatory = $true)][int]$RootProcessId)

    # ``Stop-Process`` does not terminate descendants on Windows.  Harbor
    # launches Python and Docker helpers, so stop leaves before the verified
    # root to prevent an orphaned benchmark from continuing to bill requests.
    $processes = Get-CimInstance Win32_Process
    $frontier = [System.Collections.Generic.Queue[int]]::new()
    $frontier.Enqueue($RootProcessId)
    $seen = [System.Collections.Generic.HashSet[int]]::new()
    $descendants = [System.Collections.Generic.List[int]]::new()
    while ($frontier.Count -gt 0) {
        $current = $frontier.Dequeue()
        if (-not $seen.Add($current)) {
            continue
        }
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

function Invoke-OpenTopiaTerminalBench {
    param(
        [string]$Task,
        [ValidateSet('before', 'after')]
        [string]$Snapshot
    )

    $serverBinary = if ($Snapshot -eq 'before') { $BeforeServerBinary } else { $AfterServerBinary }
    $jobsDir = if ($Snapshot -eq 'before') { $beforeJobs } else { $afterJobs }
    $stdout = Join-Path $jobsDir 'launcher.stdout.log'
    $stderr = Join-Path $jobsDir 'launcher.stderr.log'
    $arguments = @(
        'run',
        '--dataset', 'terminal-bench/terminal-bench@latest',
        '--agent', 'evaluation.integrations.harbor.opentopia_container_agent:OpenTopiaContainerAgent',
        '--agent-kwarg', "server_binary=$serverBinary",
        '--agent-kwarg', 'reasoning_effort=high',
        '--agent-kwarg', 'max_output_tokens=8192',
        '--agent-kwarg', 'run_timeout_sec=1800',
        '--env-file', $EnvFile,
        '--include-task-name', "terminal-bench/$Task",
        '--n-attempts', '1',
        '--n-concurrent', '1',
        '--env', 'docker',
        '--jobs-dir', $jobsDir,
        '--yes'
    )

    Write-RunEvent -Task $Task -Snapshot $Snapshot -Status 'started'
    $harborProcess = Start-Process -FilePath $harbor -ArgumentList $arguments `
        -WorkingDirectory (Get-Location).Path -WindowStyle Hidden `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    $completedWithinWatchdog = $harborProcess.WaitForExit($HarnessTimeoutSeconds * 1000)
    if (-not $completedWithinWatchdog) {
        Stop-ChildProcessTree -RootProcessId $harborProcess.Id
        $exitCode = 1
    } else {
        $harborProcess.Refresh()
        $exitCode = $harborProcess.ExitCode
    }
    Write-RunEvent -Task $Task -Snapshot $Snapshot -Status 'finished' -ExitCode $exitCode
    if ($exitCode -ne 0) {
        return $false
    }
    $providerFailure = Get-ProviderFailure -Task $Task -JobsDir $jobsDir
    if ($providerFailure) {
        Write-RunEvent -Task $Task -Snapshot $Snapshot -Status 'provider_failure' -ExitCode $exitCode
        return $false
    }
    return $true
}

function Invoke-OpenTopiaTerminalBenchPair {
    param(
        [string]$Task,
        [ValidateSet('before', 'after')]
        [string]$First
    )

    $second = if ($First -eq 'before') { 'after' } else { 'before' }
    # A transient provider failure in one snapshot must not discard its paired
    # observation.  Finish the other snapshot first; the coordinator will
    # later retry only the failed side.
    $firstSucceeded = Invoke-OpenTopiaTerminalBench -Task $Task -Snapshot $First
    $secondSucceeded = Invoke-OpenTopiaTerminalBench -Task $Task -Snapshot $second
    if (-not $firstSucceeded -or -not $secondSucceeded) {
        throw "Provider or launcher failure in $Task; the successful counterpart was preserved for selective retry."
    }
}

if ($WaitForPid -gt 0) {
    Wait-Process -Id $WaitForPid -ErrorAction SilentlyContinue
}

if ($OnlyPairTask) {
    if ($OnlySnapshot) {
        if (-not (Invoke-OpenTopiaTerminalBench -Task $OnlyPairTask -Snapshot $OnlySnapshot)) {
            throw "Provider or launcher failure in $OnlyPairTask/$OnlySnapshot."
        }
        return
    }
    if ($RetryProviderFailures) {
        $retryFirst = if ($OnlyPairFirst) { $OnlyPairFirst } else { 'before' }
        $retrySecond = if ($retryFirst -eq 'before') { 'after' } else { 'before' }
        $retryFailed = $false
        foreach ($snapshot in @($retryFirst, $retrySecond)) {
            $jobsDir = if ($snapshot -eq 'before') { $beforeJobs } else { $afterJobs }
            if (-not (Get-ProviderFailure -Task $OnlyPairTask -JobsDir $jobsDir)) {
                continue
            }
            if (-not (Invoke-OpenTopiaTerminalBench -Task $OnlyPairTask -Snapshot $snapshot)) {
                $retryFailed = $true
            }
        }
        if ($retryFailed) {
            throw "Provider or launcher failure remains in $OnlyPairTask; successful snapshots were preserved."
        }
        return
    }
    $onlyPairFirst = if ($OnlyPairFirst) { $OnlyPairFirst } else { 'before' }
    Invoke-OpenTopiaTerminalBenchPair -Task $OnlyPairTask -First $onlyPairFirst
    return
} elseif ($ResumePairTask) {
    Invoke-OpenTopiaTerminalBenchPair -Task $ResumePairTask -First 'before'
} elseif ($ResumeAfterTask) {
    if (-not (Invoke-OpenTopiaTerminalBench -Task $ResumeAfterTask -Snapshot 'after')) {
        throw "Provider or launcher failure in $ResumeAfterTask/after."
    }
}

# `coq-block-bound` is intentionally absent until its Docker image pull has
# passed the no-model Oracle preflight.  The first snapshot alternates by task
# rank to reduce endpoint-time drift between before and after measurements.
$remainingPairs = @(
    @{ task = 'ontology-kg-querying'; first = 'after' },
    @{ task = 'telecom-entity-resolution'; first = 'after' },
    @{ task = 'rs-archive-clone'; first = 'before' },
    @{ task = 'distributed-dedup'; first = 'after' },
    @{ task = 'bun-sourcemap-leak'; first = 'after' },
    @{ task = 'retro-console-soc'; first = 'before' },
    @{ task = 'roy-polymorph-cn'; first = 'after' },
    @{ task = 'data-anonymization'; first = 'before' },
    @{ task = 'hof-topology-interpenetration'; first = 'after' },
    @{ task = 'freecad-spring-clip'; first = 'before' },
    @{ task = 'wdm-design'; first = 'after' },
    @{ task = 'photonic-waveguide-routing'; first = 'before' },
    @{ task = 'cargo-flight-dispatch'; first = 'after' },
    @{ task = 'protein-autointerp-disulfide'; first = 'before' },
    @{ task = 'shadow-relay'; first = 'after' },
    @{ task = 'vpp-loss-divergence'; first = 'before' }
)

$startedRemaining = -not $StartRemainingTask
foreach ($pair in $remainingPairs) {
    if (-not $startedRemaining) {
        if ($pair.task -ne $StartRemainingTask) {
            continue
        }
        $startedRemaining = $true
    }
    Invoke-OpenTopiaTerminalBenchPair -Task $pair.task -First $pair.first
}

if ($StartRemainingTask -and -not $startedRemaining) {
    throw "StartRemainingTask was not found in the configured pair list: $StartRemainingTask"
}
