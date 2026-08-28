param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [string]$RunsDirectoryName = 'swebench-codex-account-default-20260828',

    [ValidateRange(1, 2)]
    [int]$MaxParallelTasks = 2,

    [ValidateRange(180, 3600)]
    [int]$RunTimeoutSeconds = 1800,

    [string[]]$ExcludeInstance = @(),

    # Controllers pass a JSON list here when resuming several completed
    # instances. pwsh -File otherwise treats later string-array values as
    # positional parameters.
    [string]$InstanceListPath = '',

    [string]$OnlyInstance = ''
)

$ErrorActionPreference = 'Stop'
$runner = Join-Path $PSScriptRoot 'run-swebench-codex-instance.ps1'
if (-not (Test-Path -LiteralPath $runner)) { throw "SWE-bench Codex single-runner is missing: $runner" }
$instances = @(
    'astropy__astropy-13579',
    'pallets__flask-5014',
    'psf__requests-1724',
    'pydata__xarray-6992',
    'pylint-dev__pylint-6903',
    'pytest-dev__pytest-6202',
    'sphinx-doc__sphinx-9258',
    'sympy__sympy-13031',
    'matplotlib__matplotlib-24627'
)
$allInstances = @($instances)
if ($InstanceListPath) {
    if ($OnlyInstance -or $ExcludeInstance.Count -gt 0) {
        throw 'InstanceListPath cannot be combined with OnlyInstance or ExcludeInstance.'
    }
    $selectedInstances = @(Get-Content -LiteralPath (Resolve-Path -LiteralPath $InstanceListPath).Path -Raw | ConvertFrom-Json)
    if ($selectedInstances.Count -eq 0 -or @($selectedInstances | Select-Object -Unique).Count -ne $selectedInstances.Count) {
        throw 'InstanceListPath must contain a nonempty, unique instance list.'
    }
    foreach ($instance in $selectedInstances) {
        if ($allInstances -notcontains $instance) { throw "InstanceListPath contains an unsupported instance: $instance" }
    }
    $instances = @($selectedInstances)
} elseif ($OnlyInstance) {
    if ($instances -notcontains $OnlyInstance) { throw "OnlyInstance is not in the selected SWE-bench set: $OnlyInstance" }
    $instances = @($OnlyInstance)
} elseif ($ExcludeInstance.Count -gt 0) {
    foreach ($instance in $ExcludeInstance) {
        if ($instances -notcontains $instance) { throw "ExcludeInstance is not in the selected SWE-bench set: $instance" }
    }
    $instances = @($instances | Where-Object { $_ -notin $ExcludeInstance })
    if ($instances.Count -eq 0) { throw 'ExcludeInstance removed every selected SWE-bench instance.' }
}

$runsRoot = Join-Path $PlanRoot $RunsDirectoryName
New-Item -ItemType Directory -Force -Path $runsRoot | Out-Null
$pending = [System.Collections.ArrayList]::new()
foreach ($instance in $instances) { [void]$pending.Add($instance) }
$active = [System.Collections.ArrayList]::new()
$completed = [System.Collections.ArrayList]::new()

while ($pending.Count -gt 0 -or $active.Count -gt 0) {
    while ($pending.Count -gt 0 -and $active.Count -lt $MaxParallelTasks) {
        $instance = $pending[0]
        $pending.RemoveAt(0)
        $instanceDir = Join-Path $runsRoot $instance
        New-Item -ItemType Directory -Force -Path $instanceDir | Out-Null
        $stdout = Join-Path $instanceDir 'launcher.stdout.log'
        $stderr = Join-Path $instanceDir 'launcher.stderr.log'
        # The host may fail to create redirected files in Start-Process.
        New-Item -ItemType File -Force -Path $stdout | Out-Null
        New-Item -ItemType File -Force -Path $stderr | Out-Null
        $arguments = @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $runner,
            '-PlanRoot', $PlanRoot,
            '-Instance', $instance,
            '-RunsDirectoryName', $RunsDirectoryName,
            '-RunTimeoutSeconds', $RunTimeoutSeconds
        )
        $process = Start-Process -FilePath 'pwsh.exe' -ArgumentList $arguments `
            -WorkingDirectory (Get-Location).Path -WindowStyle Hidden `
            -RedirectStandardOutput $stdout `
            -RedirectStandardError $stderr -PassThru
        [void]$active.Add([pscustomobject]@{
            instance = $instance
            process = $process
            startedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
        })
    }
    Start-Sleep -Seconds 2
    foreach ($entry in @($active)) {
        $entry.process.Refresh()
        if (-not $entry.process.HasExited) { continue }
        [void]$completed.Add([ordered]@{
            instance = $entry.instance
            exitCode = $entry.process.ExitCode
            startedAtUtc = $entry.startedAtUtc
            finishedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
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
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $runsRoot 'manifest.json') -Encoding utf8

if (@($completed | Where-Object { $_.exitCode -ne 0 }).Count -gt 0) {
    throw 'One or more Codex SWE-bench task launchers failed. Check per-instance logs before retrying.'
}
