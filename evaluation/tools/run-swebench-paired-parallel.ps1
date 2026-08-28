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
    [string]$BeforeRunsDirectoryName,

    [Parameter(Mandatory = $true)]
    [string]$AfterRunsDirectoryName,

    [ValidateRange(1, 4)]
    [int]$MaxParallelPairs = 2,

    [string]$StartRemainingInstance = ''
)

$ErrorActionPreference = 'Stop'
$pairRunner = Join-Path $PSScriptRoot 'run-swebench-paired.ps1'
if (-not (Test-Path -LiteralPath $pairRunner)) {
    throw 'The sequential SWE-bench pair runner is missing.'
}

# Each child gets a distinct before/after directory and launcher logs. A pair
# itself always stays ordered, so its two snapshots never execute together.
$pairs = @(
    @{ instance = 'astropy__astropy-13579'; first = 'after' },
    @{ instance = 'pallets__flask-5014'; first = 'before' },
    @{ instance = 'psf__requests-1724'; first = 'after' },
    @{ instance = 'pydata__xarray-6992'; first = 'before' },
    @{ instance = 'pylint-dev__pylint-6903'; first = 'after' },
    @{ instance = 'pytest-dev__pytest-6202'; first = 'before' },
    @{ instance = 'sphinx-doc__sphinx-9258'; first = 'after' },
    @{ instance = 'sympy__sympy-13031'; first = 'before' },
    @{ instance = 'matplotlib__matplotlib-24627'; first = 'after' }
)

if ($StartRemainingInstance) {
    $startIndex = -1
    for ($i = 0; $i -lt $pairs.Count; $i++) {
        if ($pairs[$i].instance -eq $StartRemainingInstance) {
            $startIndex = $i
            break
        }
    }
    if ($startIndex -lt 0) {
        throw "StartRemainingInstance was not found in the configured pair list: $StartRemainingInstance"
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
    while (-not $failed -and $pending.Count -gt 0 -and $active.Count -lt $MaxParallelPairs) {
        $pair = $pending[0]
        $pending.RemoveAt(0)
        $instanceSlug = $pair.instance -replace '[^A-Za-z0-9._-]', '-'
        $beforeRuns = Join-Path $BeforeRunsDirectoryName $instanceSlug
        $afterRuns = Join-Path $AfterRunsDirectoryName $instanceSlug
        $stdout = Join-Path $PlanRoot "swebench-parallel-$instanceSlug.stdout.log"
        $stderr = Join-Path $PlanRoot "swebench-parallel-$instanceSlug.stderr.log"
        $runLog = "swebench-parallel-$instanceSlug.run-log.jsonl"
        $arguments = @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $pairRunner,
            '-PlanRoot', $PlanRoot,
            '-BeforeServerBinary', $BeforeServerBinary,
            '-AfterServerBinary', $AfterServerBinary,
            '-EnvFile', $EnvFile,
            '-BeforeRunsDirectoryName', $beforeRuns,
            '-AfterRunsDirectoryName', $afterRuns,
            '-RunLogFileName', $runLog,
            '-OnlyPairInstance', $pair.instance,
            '-OnlyPairFirst', $pair.first
        )
        $process = Start-Process -FilePath 'pwsh.exe' -ArgumentList $arguments -WorkingDirectory (Get-Location).Path -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
        [void]$active.Add([pscustomobject]@{
            instance = $pair.instance
            process = $process
            beforeRuns = $beforeRuns
            afterRuns = $afterRuns
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
            instance = $entry.instance
            exitCode = $exitCode
            startedAtUtc = $entry.startedAtUtc
            finishedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
            beforeRuns = $entry.beforeRuns
            afterRuns = $entry.afterRuns
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
    pendingSkippedAfterFailure = @($pending | ForEach-Object { $_.instance })
}
$manifestPath = Join-Path $PlanRoot 'swebench-parallel-manifest.json'
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8

if ($failed) {
    throw 'One or more parallel SWE-bench pairs failed; no further pairs were started.'
}
