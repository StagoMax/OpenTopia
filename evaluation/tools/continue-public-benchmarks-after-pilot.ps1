param(
    [Parameter(Mandatory = $true)]
    [int]$PilotRunnerPid,

    [string]$PilotRunLogFile = 'terminal-bench-schema-compat-pilot-mp-20260827-run-log.jsonl',

    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [string]$BeforeServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$AfterServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$EnvFile,

    [ValidateRange(1, 4)]
    [int]$MaxParallelPairs = 2,

    [string]$StartRemainingTerminalTask = '',

    [string]$TerminalBeforeJobsDirectoryName = 'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827',

    [string]$TerminalAfterJobsDirectoryName = 'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827',

    [string[]]$TerminalBeforeSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-pilot-mp-20260827',
        'terminal-bench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827'
    ),

    [string[]]$TerminalAfterSummaryDirectoryNames = @(
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-pilot-mp-20260827-clean',
        'terminal-bench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827'
    )
)

$ErrorActionPreference = 'Stop'
$terminalRunner = Join-Path $PSScriptRoot 'run-terminal-bench-paired-parallel.ps1'
$sweRunner = Join-Path $PSScriptRoot 'run-swebench-paired-parallel.ps1'
$terminalSummarizer = Join-Path $PSScriptRoot 'summarize-terminal-bench-paired.py'
$sweSummarizer = Join-Path $PSScriptRoot 'summarize-swebench-paired.py'
$reportGenerator = Join-Path $PSScriptRoot 'generate-evaluation-report.py'
$python = (Get-Command python.exe -ErrorAction Stop).Source
$controlLog = Join-Path $PlanRoot 'public-benchmarks-continuation-20260827.jsonl'
$pilotRunLog = if ([System.IO.Path]::IsPathRooted($PilotRunLogFile)) { $PilotRunLogFile } else { Join-Path $PlanRoot $PilotRunLogFile }
$parallelismOverride = Join-Path $PlanRoot 'evaluation-parallelism-override.json'
$terminalPrelaunchMarker = Join-Path $PlanRoot 'terminal-bench-prelaunch-20260827.json'

# A recovery controller can hand off to this script after it has already been
# started with a conservative parallelism value.  Read a bounded, durable
# override at that hand-off so the remaining independent pairs can be sped up
# without interrupting or duplicating the recovery pair itself.
if (Test-Path -LiteralPath $parallelismOverride) {
    try {
        $override = Get-Content -LiteralPath $parallelismOverride -Raw | ConvertFrom-Json
        $candidate = [int]$override.maxParallelPairs
    } catch {
        throw "Parallelism override could not be read: $parallelismOverride"
    }
    if ($candidate -lt 1 -or $candidate -gt 4) {
        throw "Parallelism override must be between 1 and 4: $parallelismOverride"
    }
    $MaxParallelPairs = $candidate
}

foreach ($path in @($terminalRunner, $sweRunner, $terminalSummarizer, $sweSummarizer, $reportGenerator, $BeforeServerBinary, $AfterServerBinary, $EnvFile, $pilotRunLog)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required continuation input is missing: $path"
    }
}

function Write-ControlEvent {
    param([string]$Stage, [string]$Status, [int]$ExitCode = -1)

    [ordered]@{
        timestamp = (Get-Date).ToUniversalTime().ToString('o')
        stage = $Stage
        status = $Status
        exitCode = $ExitCode
        maxParallelPairs = $MaxParallelPairs
    } | ConvertTo-Json -Compress | Add-Content -LiteralPath $controlLog -Encoding utf8
}

function Invoke-PythonStage {
    param(
        [string]$Stage,
        [string]$Program,
        [string[]]$ProgramArguments
    )

    Write-ControlEvent -Stage $Stage -Status 'started'
    & $python $Program @ProgramArguments
    $exitCode = $LASTEXITCODE
    Write-ControlEvent -Stage $Stage -Status 'finished' -ExitCode $exitCode
    if ($exitCode -ne 0) {
        throw "$Stage failed with exit code $exitCode."
    }
}

function Test-ValidPilotCompletion {
    # Do not rely on Process.ExitCode for a process that this PowerShell
    # instance did not create: it can be null after exit. The sequential
    # runner records the authoritative completion and provider-failure events.
    $events = @()
    foreach ($line in Get-Content -LiteralPath $pilotRunLog -ErrorAction Stop) {
        try {
            $events += $line | ConvertFrom-Json
        } catch {
            return $false
        }
    }
    foreach ($snapshot in @('before', 'after')) {
        $finished = @($events | Where-Object {
            $_.snapshot -eq $snapshot -and $_.status -eq 'finished' -and $_.exitCode -eq 0
        })
        if ($finished.Count -eq 0) {
            return $false
        }
        if (@($events | Where-Object { $_.snapshot -eq $snapshot -and $_.status -eq 'provider_failure' }).Count -gt 0) {
            return $false
        }
    }
    return $true
}

# Wait when the pilot is still active; a continuation restarted after the
# pilot exited validates the same durable event log immediately.
$pilot = if ($PilotRunnerPid -gt 0) {
    Get-Process -Id $PilotRunnerPid -ErrorAction SilentlyContinue
} else {
    $null
}
if ($pilot) {
    Write-ControlEvent -Stage 'pilot' -Status 'waiting'
    $pilot.WaitForExit()
} else {
    Write-ControlEvent -Stage 'pilot' -Status 'recovery_validation_started'
}
if (-not (Test-ValidPilotCompletion)) {
    Write-ControlEvent -Stage 'pilot' -Status 'failed_validation' -ExitCode 1
    throw 'Pilot completion was not valid; no queued benchmark was started.'
}
Write-ControlEvent -Stage 'pilot' -Status 'validated' -ExitCode 0

$terminalArguments = @{
    PlanRoot = $PlanRoot
    BeforeServerBinary = $BeforeServerBinary
    AfterServerBinary = $AfterServerBinary
    EnvFile = $EnvFile
    BeforeJobsDirectoryName = $TerminalBeforeJobsDirectoryName
    AfterJobsDirectoryName = $TerminalAfterJobsDirectoryName
    MaxParallelPairs = $MaxParallelPairs
}
if ($StartRemainingTerminalTask) {
    $terminalArguments.StartRemainingTask = $StartRemainingTerminalTask
}
Write-ControlEvent -Stage 'terminal_bench' -Status 'started'
try {
    # An acceleration controller may have started these independent pairs while
    # a recovery pair was still completing.  Reuse that exact run instead of
    # launching the same tasks again, which would both corrupt job directories
    # and charge the provider twice.
    $usedPrelaunch = $false
    if (Test-Path -LiteralPath $terminalPrelaunchMarker) {
        $prelaunch = Get-Content -LiteralPath $terminalPrelaunchMarker -Raw | ConvertFrom-Json
        $expectedStart = if ($StartRemainingTerminalTask) { $StartRemainingTerminalTask } else { '' }
        if ($prelaunch.startRemainingTask -ne $expectedStart -or
            $prelaunch.beforeJobsDirectoryName -ne $TerminalBeforeJobsDirectoryName -or
            $prelaunch.afterJobsDirectoryName -ne $TerminalAfterJobsDirectoryName -or
            [int]$prelaunch.expectedTaskCount -lt 1) {
            throw "Terminal prelaunch marker does not match this continuation: $terminalPrelaunchMarker"
        }
        $prelaunchProcess = Get-Process -Id ([int]$prelaunch.processId) -ErrorAction SilentlyContinue
        if ($null -ne $prelaunchProcess) {
            $prelaunchProcess.WaitForExit()
        }
        $manifestFile = if ($prelaunch.completionManifestFile -is [string] -and $prelaunch.completionManifestFile) {
            $prelaunch.completionManifestFile
        } else {
            'terminal-bench-parallel-manifest.json'
        }
        $manifestPath = if ([System.IO.Path]::IsPathRooted($manifestFile)) {
            $manifestFile
        } else {
            Join-Path $PlanRoot $manifestFile
        }
        if (-not (Test-Path -LiteralPath $manifestPath)) {
            throw "Terminal prelaunch ended without its dispatcher manifest: $manifestPath"
        }
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $completed = @($manifest.completed)
        $pending = @($manifest.pendingSkippedAfterFailure)
        if ($completed.Count -ne [int]$prelaunch.expectedTaskCount -or
            $pending.Count -ne 0 -or
            @($completed | Where-Object { [int]$_.exitCode -ne 0 }).Count -ne 0) {
            throw 'Accelerated Terminal-Bench prelaunch was incomplete or failed; refusing to create duplicate trials.'
        }
        $usedPrelaunch = $true
    }
    if (-not $usedPrelaunch) {
        & $terminalRunner @terminalArguments
    }
    $terminalExitCode = 0
} catch {
    $terminalExitCode = 1
    Write-ControlEvent -Stage 'terminal_bench' -Status 'finished' -ExitCode $terminalExitCode
    throw
}
Write-ControlEvent -Stage 'terminal_bench' -Status 'finished' -ExitCode $terminalExitCode
if ($terminalExitCode -ne 0) {
    throw "Terminal-Bench dispatcher failed with exit code $terminalExitCode; SWE-bench was not started."
}

$sweArguments = @{
    PlanRoot = $PlanRoot
    BeforeServerBinary = $BeforeServerBinary
    AfterServerBinary = $AfterServerBinary
    EnvFile = $EnvFile
    BeforeRunsDirectoryName = 'swebench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827'
    AfterRunsDirectoryName = 'swebench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827'
    MaxParallelPairs = $MaxParallelPairs
}
Write-ControlEvent -Stage 'swe_bench' -Status 'started'
try {
    & $sweRunner @sweArguments
    $sweExitCode = 0
} catch {
    $sweExitCode = 1
    Write-ControlEvent -Stage 'swe_bench' -Status 'finished' -ExitCode $sweExitCode
    throw
}
Write-ControlEvent -Stage 'swe_bench' -Status 'finished' -ExitCode $sweExitCode
if ($sweExitCode -ne 0) {
    throw "SWE-bench dispatcher failed with exit code $sweExitCode."
}

# Summaries intentionally consume only paired metadata. The Terminal-Bench
# roots include the valid pilot plus the parallel remainder; the SWE-bench
# roots contain its full parallel run.
$terminalSummaryDir = Join-Path $PlanRoot 'final-terminal-bench-schemafix-20260827'
$sweSummaryDir = Join-Path $PlanRoot 'final-swebench-schemafix-20260827'
$reportPath = Join-Path $PlanRoot 'FINAL-BEFORE-AFTER-REPORT-20260827.md'
$terminalSummaryArguments = @()
foreach ($directory in $TerminalBeforeSummaryDirectoryNames) {
    $terminalSummaryArguments += '--before-root', (Join-Path $PlanRoot $directory)
}
foreach ($directory in $TerminalAfterSummaryDirectoryNames) {
    $terminalSummaryArguments += '--after-root', (Join-Path $PlanRoot $directory)
}
$terminalSummaryArguments += '--output-dir', $terminalSummaryDir
Invoke-PythonStage -Stage 'terminal_summary' -Program $terminalSummarizer -ProgramArguments $terminalSummaryArguments

$sweSummaryArguments = @(
    '--before-root', (Join-Path $PlanRoot 'swebench-gpt56terra-high-before-6d52-schema-compat-shim-full-par2-20260827'),
    '--after-root', (Join-Path $PlanRoot 'swebench-gpt56terra-high-after-404e596-schemafix-worktree-full-par2-20260827'),
    '--output-dir', $sweSummaryDir
)
Invoke-PythonStage -Stage 'swe_summary' -Program $sweSummarizer -ProgramArguments $sweSummaryArguments

$reportArguments = @(
    '--terminal-summary', (Join-Path $terminalSummaryDir 'summary.json'),
    '--swe-summary', (Join-Path $sweSummaryDir 'summary.json'),
    '--output', $reportPath
)
Invoke-PythonStage -Stage 'final_report' -Program $reportGenerator -ProgramArguments $reportArguments
