param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [ValidateRange(1, 4)]
    [int]$MaxConcurrent = 2
)

$ErrorActionPreference = 'Stop'

# The official harness writes intermediate patches and test output.  Its
# default text encoding must be UTF-8 on this Windows host; otherwise a valid
# Unicode diff or test log can abort scoring after the tests already ran.
$env:PYTHONUTF8 = '1'

$python = Join-Path $PlanRoot 'runtime\swebench-py311\Scripts\python.exe'
$instances = Join-Path $PlanRoot 'swebench-verified-selected-instances-78f471bf.jsonl'
$scorer = Join-Path $PSScriptRoot '..\integrations\swebench\score_opentopia_prediction.py'
foreach ($path in @($python, $instances, $scorer)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required rescore input is missing: $path"
    }
}

$roots = @{
    before = Join-Path $PlanRoot 'swebench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827'
    after = Join-Path $PlanRoot 'swebench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827'
}
$instancesToScore = @(
    'astropy__astropy-13579',
    'matplotlib__matplotlib-24627',
    'pallets__flask-5014',
    'psf__requests-1724',
    'pydata__xarray-6992',
    'pylint-dev__pylint-6903',
    'pytest-dev__pytest-6202',
    'sphinx-doc__sphinx-9258',
    'sympy__sympy-13031'
)

function Test-ValidFixedScore {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Instance
    )

    try {
        $payload = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
        $result = @($payload.result)
        return $result.Count -eq 2 -and $null -ne $result[1].$Instance
    } catch {
        return $false
    }
}

$queue = [System.Collections.Generic.Queue[object]]::new()
foreach ($snapshot in @('before', 'after')) {
    foreach ($instance in $instancesToScore) {
        $runDirectory = Join-Path (Join-Path $roots[$snapshot] $instance) $instance
        $prediction = Join-Path $runDirectory 'prediction.jsonl'
        if (-not (Test-Path -LiteralPath $prediction)) {
            throw "Missing prediction for $snapshot/${instance}: $prediction"
        }
        $queue.Enqueue([pscustomobject]@{
            Snapshot = $snapshot
            Instance = $instance
            Prediction = $prediction
            Output = Join-Path $runDirectory 'official-score-lf-fixed.json'
            Stdout = Join-Path $runDirectory 'score-lf-fixed.stdout.log'
            Stderr = Join-Path $runDirectory 'score-lf-fixed.stderr.log'
        })
    }
}

$running = [System.Collections.Generic.List[object]]::new()
$completed = [System.Collections.Generic.List[object]]::new()
while ($queue.Count -gt 0 -or $running.Count -gt 0) {
    while ($queue.Count -gt 0 -and $running.Count -lt $MaxConcurrent) {
        $item = $queue.Dequeue()
        if ((Test-Path -LiteralPath $item.Output) -and (Test-ValidFixedScore -Path $item.Output -Instance $item.Instance)) {
            Write-Output "reused $($item.Snapshot)/$($item.Instance)"
            $completed.Add([pscustomobject]@{ Item = $item; ExitCode = 0; Reused = $true })
            continue
        }
        $arguments = @(
            $scorer,
            '--instances', $instances,
            '--instance-id', $item.Instance,
            '--prediction', $item.Prediction,
            '--run-id', "opentopia-$($item.Snapshot)-$($item.Instance)-lf-fixed",
            '--timeout-seconds', '1800',
            '--output', $item.Output
        )
        $process = Start-Process -FilePath $python -ArgumentList $arguments -WorkingDirectory (Get-Location).Path `
            -WindowStyle Hidden -RedirectStandardOutput $item.Stdout -RedirectStandardError $item.Stderr -PassThru
        $running.Add([pscustomobject]@{ Item = $item; Process = $process })
        Write-Output "started $($item.Snapshot)/$($item.Instance) pid=$($process.Id)"
    }

    foreach ($entry in @($running)) {
        if (-not $entry.Process.HasExited) {
            continue
        }
        $entry.Process.Refresh()
        $completed.Add([pscustomobject]@{ Item = $entry.Item; ExitCode = $entry.Process.ExitCode; Reused = $false })
        [void]$running.Remove($entry)
        Write-Output "finished $($entry.Item.Snapshot)/$($entry.Item.Instance) exit=$($entry.Process.ExitCode)"
    }
    if ($running.Count -gt 0) {
        Start-Sleep -Seconds 1
    }
}

$failed = @($completed | Where-Object { $_.ExitCode -ne 0 })
if ($failed.Count -gt 0) {
    $failedNames = ($failed | ForEach-Object { "$($_.Item.Snapshot)/$($_.Item.Instance)" }) -join ', '
    throw "LF-preserving SWE rescore failed for: $failedNames"
}
Write-Output "completed=$($completed.Count)"
