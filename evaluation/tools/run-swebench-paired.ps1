param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [string]$BeforeServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$AfterServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$EnvFile,

    [string]$BeforeRunsDirectoryName = 'swebench-unbounded-before-6d52',

    [string]$AfterRunsDirectoryName = 'swebench-unbounded-after-current-404e596',

    [string]$RunLogFileName = 'swebench-unbounded-current-run-log.jsonl',

    # Resume from a task without replaying the previously completed pairs.
    [string]$StartRemainingInstance = '',

    # Run precisely one before/after pair and then exit.  The parallel
    # dispatcher uses this to isolate each instance's artifacts and failure.
    [string]$OnlyPairInstance = '',

    [ValidateSet('', 'before', 'after')]
    [string]$OnlyPairFirst = '',

    [int]$WaitForPid = 0
)

$ErrorActionPreference = 'Stop'

$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$python = Join-Path $PlanRoot 'runtime\swebench-py311\Scripts\python.exe'
$instances = Join-Path $PlanRoot 'swebench-verified-selected-instances-78f471bf.jsonl'
$agentRunner = Join-Path $sourceRoot 'evaluation\integrations\swebench\run_opentopia_instance.py'
$officialScorer = Join-Path $sourceRoot 'evaluation\integrations\swebench\score_opentopia_prediction.py'
$beforeRuns = Join-Path $PlanRoot $BeforeRunsDirectoryName
$afterRuns = Join-Path $PlanRoot $AfterRunsDirectoryName
$runLog = Join-Path $PlanRoot $RunLogFileName

foreach ($path in @($python, $instances, $agentRunner, $officialScorer, $BeforeServerBinary, $AfterServerBinary, $EnvFile)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required evaluation input is missing: $path"
    }
}

foreach ($directory in @($beforeRuns, $afterRuns)) {
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
}

$env:PYTHONUTF8 = '1'
$env:PYTHONPATH = $sourceRoot

function Write-RunEvent {
    param(
        [string]$Instance,
        [ValidateSet('before', 'after')]
        [string]$Snapshot,
        [ValidateSet('agent_started', 'agent_finished', 'score_started', 'score_finished', 'score_skipped', 'provider_failure')]
        [string]$Status,
        [int]$ExitCode = -1
    )

    [pscustomobject]@{
        timestamp = (Get-Date).ToUniversalTime().ToString('o')
        instanceId = $Instance
        snapshot = $Snapshot
        stage = $Status
        exitCode = $ExitCode
        rolloutLimitTokens = $null
        model = 'gpt-5.6-terra'
        reasoningEffort = 'high'
        maxOutputTokens = 8192
        agentTimeoutSeconds = 1800
        officialScoreTimeoutSeconds = 1800
    } | ConvertTo-Json -Compress | Add-Content -LiteralPath $runLog -Encoding utf8
}

function Get-ProviderFailure {
    param(
        [string]$Instance,
        [string]$RunsRoot
    )

    # A rejected provider request can still leave the Python runner with a
    # successful process exit. Read only its terminal-error metadata, never
    # benchmark instructions, model output, patches, or tool payloads.
    $recordPath = Join-Path (Join-Path $RunsRoot $Instance) 'prediction.run.json'
    if (-not (Test-Path -LiteralPath $recordPath)) {
        return 'Agent runner finished without a readable run record.'
    }
    try {
        $record = Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json
    } catch {
        return 'Agent runner produced an unreadable run record.'
    }
    $error = $record.controlledSettings.turnError
    if ($error -isnot [string]) {
        return $null
    }
    $normalized = $error.ToLowerInvariant()
    if (
        $normalized.Contains('provider request failed') -or
        $normalized.Contains('provider stream returned an error') -or
        $normalized.Contains('error sending request') -or
        $normalized.Contains('error decoding response body') -or
        $normalized.Contains('upstream_error')
    ) {
        return $error
    }
    return $null
}

function Invoke-OpenTopiaSweBench {
    param(
        [string]$Instance,
        [ValidateSet('before', 'after')]
        [string]$Snapshot
    )

    $serverBinary = if ($Snapshot -eq 'before') { $BeforeServerBinary } else { $AfterServerBinary }
    $runsRoot = if ($Snapshot -eq 'before') { $beforeRuns } else { $afterRuns }
    $instanceDirectory = Join-Path $runsRoot $Instance
    $logsDirectory = Join-Path $instanceDirectory 'agent-logs'
    $prediction = Join-Path $instanceDirectory 'prediction.jsonl'
    $runLabel = "opentopia-$Snapshot-$Instance"
    $score = Join-Path $instanceDirectory 'official-score.json'
    $agentStdout = Join-Path $instanceDirectory 'agent.stdout.log'
    $agentStderr = Join-Path $instanceDirectory 'agent.stderr.log'
    $scoreStdout = Join-Path $instanceDirectory 'score.stdout.log'
    $scoreStderr = Join-Path $instanceDirectory 'score.stderr.log'

    New-Item -ItemType Directory -Force -Path $instanceDirectory | Out-Null

    $agentArguments = @(
        $agentRunner,
        '--instances', $instances,
        '--instance-id', $Instance,
        '--server-binary', $serverBinary,
        '--env-file', $EnvFile,
        '--output', $prediction,
        '--logs-dir', $logsDirectory,
        '--run-label', $runLabel,
        '--reasoning-effort', 'high',
        '--max-output-tokens', '8192',
        '--run-timeout-seconds', '1800'
    )

    Write-RunEvent -Instance $Instance -Snapshot $Snapshot -Status 'agent_started'
    & $python @agentArguments 1>> $agentStdout 2>> $agentStderr
    $agentExitCode = $LASTEXITCODE
    Write-RunEvent -Instance $Instance -Snapshot $Snapshot -Status 'agent_finished' -ExitCode $agentExitCode
    if ($agentExitCode -ne 0) {
        return $false
    }
    $providerFailure = Get-ProviderFailure -Instance $Instance -RunsRoot $runsRoot
    if ($providerFailure) {
        Write-RunEvent -Instance $Instance -Snapshot $Snapshot -Status 'provider_failure' -ExitCode $agentExitCode
        return $false
    }

    if (-not (Test-Path -LiteralPath $prediction)) {
        Write-RunEvent -Instance $Instance -Snapshot $Snapshot -Status 'score_skipped' -ExitCode $agentExitCode
        return $false
    }

    $scoreArguments = @(
        $officialScorer,
        '--instances', $instances,
        '--instance-id', $Instance,
        '--prediction', $prediction,
        '--run-id', $runLabel,
        '--timeout-seconds', '1800',
        '--output', $score
    )
    Write-RunEvent -Instance $Instance -Snapshot $Snapshot -Status 'score_started'
    & $python @scoreArguments 1>> $scoreStdout 2>> $scoreStderr
    $scoreExitCode = $LASTEXITCODE
    Write-RunEvent -Instance $Instance -Snapshot $Snapshot -Status 'score_finished' -ExitCode $scoreExitCode
    return ($scoreExitCode -eq 0)
}

function Invoke-OpenTopiaSweBenchPair {
    param(
        [string]$Instance,
        [ValidateSet('before', 'after')]
        [string]$First
    )

    $second = if ($First -eq 'before') { 'after' } else { 'before' }
    if (-not (Invoke-OpenTopiaSweBench -Instance $Instance -Snapshot $First)) {
        throw "Provider, agent, or launcher failure in $Instance/$First; queue stopped before $Instance/$second."
    }
    if (-not (Invoke-OpenTopiaSweBench -Instance $Instance -Snapshot $second)) {
        throw "Provider, agent, or launcher failure in $Instance/$second; queue stopped before the next pair."
    }
}

if ($WaitForPid -gt 0) {
    Wait-Process -Id $WaitForPid -ErrorAction SilentlyContinue
}

if ($OnlyPairInstance) {
    $onlyPairFirst = if ($OnlyPairFirst) { $OnlyPairFirst } else { 'before' }
    Invoke-OpenTopiaSweBenchPair -Instance $OnlyPairInstance -First $onlyPairFirst
    return
}

# The first snapshot alternates to reduce endpoint-time drift.  Every snapshot
# uses its identical instance image and the same external model configuration.
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

$startedRemaining = -not $StartRemainingInstance
foreach ($pair in $pairs) {
    if (-not $startedRemaining) {
        if ($pair.instance -ne $StartRemainingInstance) {
            continue
        }
        $startedRemaining = $true
    }
    Invoke-OpenTopiaSweBenchPair -Instance $pair.instance -First $pair.first
}

if ($StartRemainingInstance -and -not $startedRemaining) {
    throw "StartRemainingInstance was not found in the configured pair list: $StartRemainingInstance"
}
