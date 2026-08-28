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
    [string]$BeforeJobsDirectoryName,

    [Parameter(Mandatory = $true)]
    [string]$AfterJobsDirectoryName,

    [ValidateRange(1, 4)]
    [int]$MaxParallelPairs = 2,

    [string]$StartRemainingTask = '',

    # Resume a partially completed queue without repeating snapshots that
    # already produced a non-provider terminal result.
    [switch]$RetryProviderFailures
)

$ErrorActionPreference = 'Stop'
$pairRunner = Join-Path $PSScriptRoot 'run-terminal-bench-paired.ps1'
if (-not (Test-Path -LiteralPath $pairRunner)) {
    throw 'The sequential pair runner is missing.'
}

# Coq is deliberately left out until its separate no-model Docker preflight
# succeeds. Every child owns distinct job and launcher-log directories, so
# concurrent pairs cannot corrupt one another's Harbor state.
$pairs = @(
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

if ($StartRemainingTask) {
    $startIndex = [Array]::FindIndex($pairs, [Predicate[object]]{ param($pair) $pair.task -eq $StartRemainingTask })
    if ($startIndex -lt 0) {
        throw "StartRemainingTask was not found in the configured pair list: $StartRemainingTask"
    }
    $pairs = @($pairs[$startIndex..($pairs.Count - 1)])
}

$pending = [System.Collections.ArrayList]::new()
foreach ($pair in $pairs) {
    [void]$pending.Add($pair)
}
$active = [System.Collections.ArrayList]::new()
$completed = [System.Collections.ArrayList]::new()
$failed = $false

while ($pending.Count -gt 0 -or $active.Count -gt 0) {
    # Keep the worker pool full even if a pair reports an infrastructure
    # failure.  The caller receives a non-zero exit after the whole batch so
    # it can selectively retry only invalid snapshots, without idling slots.
    while ($pending.Count -gt 0 -and $active.Count -lt $MaxParallelPairs) {
        $pair = $pending[0]
        $pending.RemoveAt(0)
        $taskSlug = $pair.task -replace '[^A-Za-z0-9._-]', '-'
        $beforeJobs = Join-Path $BeforeJobsDirectoryName $taskSlug
        $afterJobs = Join-Path $AfterJobsDirectoryName $taskSlug
        $stdout = Join-Path $PlanRoot "terminal-bench-parallel-$taskSlug.stdout.log"
        $stderr = Join-Path $PlanRoot "terminal-bench-parallel-$taskSlug.stderr.log"
        $runLog = "terminal-bench-parallel-$taskSlug.run-log.jsonl"
        $arguments = @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $pairRunner,
            '-PlanRoot', $PlanRoot,
            '-BeforeServerBinary', $BeforeServerBinary,
            '-AfterServerBinary', $AfterServerBinary,
            '-EnvFile', $EnvFile,
            '-BeforeJobsDirectoryName', $beforeJobs,
            '-AfterJobsDirectoryName', $afterJobs,
            '-RunLogFileName', $runLog,
            '-OnlyPairTask', $pair.task,
            '-OnlyPairFirst', $pair.first
        )
        if ($RetryProviderFailures) {
            $arguments += '-RetryProviderFailures'
        }
        $process = Start-Process -FilePath 'pwsh.exe' -ArgumentList $arguments -WorkingDirectory (Get-Location).Path -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
        [void]$active.Add([pscustomobject]@{
            task = $pair.task
            process = $process
            beforeJobs = $beforeJobs
            afterJobs = $afterJobs
            startedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
        })
    }

    if ($active.Count -eq 0) {
        if ($failed) {
            break
        }
        continue
    }
    Start-Sleep -Seconds 2
    foreach ($entry in @($active)) {
        $entry.process.Refresh()
        if (-not $entry.process.HasExited) {
            continue
        }
        $exitCode = $entry.process.ExitCode
        [void]$completed.Add([ordered]@{
            task = $entry.task
            exitCode = $exitCode
            startedAtUtc = $entry.startedAtUtc
            finishedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
            beforeJobs = $entry.beforeJobs
            afterJobs = $entry.afterJobs
        })
        [void]$active.Remove($entry)
        if ($exitCode -ne 0) {
            $failed = $true
        }
    }
}

$manifest = [ordered]@{
    schemaVersion = 1
    maxParallelPairs = $MaxParallelPairs
    completed = @($completed)
    pendingSkippedAfterFailure = @($pending | ForEach-Object { $_.task })
}
$manifestPath = Join-Path $PlanRoot 'terminal-bench-parallel-manifest.json'
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8

if ($failed) {
    throw 'One or more parallel pairs failed; no further pairs were started.'
}
