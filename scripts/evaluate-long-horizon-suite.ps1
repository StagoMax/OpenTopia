param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$ExpectedModel = "glm-5.2",
  [string[]]$TaskManifests = @(
    "scripts\fixtures\long-horizon\task.json",
    "scripts\fixtures\long-horizon\config-migration\task.json",
    "scripts\fixtures\long-horizon\dependency-planner\task.json"
  ),
  [ValidateRange(1, 10)][int]$Repetitions = 1,
  [int]$StartPort = 8812,
  [string]$SummaryPath = "",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$runner = Join-Path $PSScriptRoot "evaluate-long-horizon.ps1"
$suiteId = "long-horizon-suite-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
$suiteRoot = Join-Path $repoRoot ".opentopia\evaluations\$suiteId"
New-Item -ItemType Directory -Path $suiteRoot -Force | Out-Null

$startedAt = Get-Date
$runs = @()
$runIndex = 0
$buildCompleted = $SkipBuild.IsPresent

foreach ($manifest in $TaskManifests) {
  $manifestPath = if ([IO.Path]::IsPathRooted($manifest)) {
    $manifest
  } else {
    Join-Path $repoRoot $manifest
  }
  $manifestPath = (Resolve-Path -LiteralPath $manifestPath).Path
  $task = Get-Content -Raw -Encoding UTF8 -LiteralPath $manifestPath | ConvertFrom-Json

  for ($repetition = 1; $repetition -le $Repetitions; $repetition += 1) {
    $runIndex += 1
    $port = $StartPort + $runIndex - 1
    $taskId = [string]$task.id
    $safeTaskId = ($taskId.ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')
    $childSummary = Join-Path $suiteRoot "$safeTaskId-$repetition.json"
    $arguments = @(
      "-NoProfile",
      "-ExecutionPolicy", "Bypass",
      "-File", $runner,
      "-EnvFile", $EnvFile,
      "-Profile", $Profile,
      "-ExpectedModel", $ExpectedModel,
      "-TaskManifest", $manifestPath,
      "-Port", $port,
      "-SummaryPath", $childSummary
    )
    if ($buildCompleted) {
      $arguments += "-SkipBuild"
    }

    $childStdout = Join-Path $suiteRoot "$safeTaskId-$repetition.stdout.log"
    $childStderr = Join-Path $suiteRoot "$safeTaskId-$repetition.stderr.log"
    $process = Start-Process `
      -FilePath "powershell.exe" `
      -ArgumentList $arguments `
      -RedirectStandardOutput $childStdout `
      -RedirectStandardError $childStderr `
      -WindowStyle Hidden `
      -Wait `
      -PassThru
    $exitCode = $process.ExitCode
    $output = @()
    if (Test-Path -LiteralPath $childStdout) {
      $output += Get-Content -Encoding UTF8 -LiteralPath $childStdout
    }
    if (Test-Path -LiteralPath $childStderr) {
      $output += Get-Content -Encoding UTF8 -LiteralPath $childStderr
    }
    if (Test-Path -LiteralPath $childSummary) {
      $result = Get-Content -Raw -Encoding UTF8 -LiteralPath $childSummary | ConvertFrom-Json
      $runs += [PSCustomObject]@{
        taskId = $taskId
        repetition = $repetition
        exitCode = $exitCode
        result = $result
        runnerOutput = $null
      }
      if ($result.artifacts.runDirectory) {
        $buildCompleted = $true
      }
    } else {
      $safeOutput = (($output | ForEach-Object { $_.ToString() }) -join "`n")
      if ($safeOutput.Length -gt 2000) {
        $safeOutput = $safeOutput.Substring($safeOutput.Length - 2000)
      }
      $runs += [PSCustomObject]@{
        taskId = $taskId
        repetition = $repetition
        exitCode = $exitCode
        result = $null
        runnerOutput = $safeOutput
      }
    }
  }
}

$validRuns = @($runs | Where-Object { $null -ne $_.result })
$passedRuns = @($validRuns | Where-Object { $_.result.status -eq "passed" })
$taskSummaries = @($validRuns | Group-Object taskId | ForEach-Object {
  $taskRuns = @($_.Group)
  $taskPassed = @($taskRuns | Where-Object { $_.result.status -eq "passed" })
  [ordered]@{
    taskId = $_.Name
    validRuns = $taskRuns.Count
    passedRuns = $taskPassed.Count
    passRate = if ($taskRuns.Count -gt 0) {
      [double]$taskPassed.Count / [double]$taskRuns.Count
    } else { $null }
    runs = @($taskRuns | ForEach-Object {
      [ordered]@{
        repetition = $_.repetition
        runId = $_.result.runId
        status = $_.result.status
        totalMs = $_.result.timing.totalMs
        totalTokens = $_.result.trajectoryMetrics.totalTokens
        completionToolCalls = $_.result.trajectoryMetrics.completionToolCalls
        verifiedPlanCompletionCalls = $_.result.trajectoryMetrics.verifiedPlanCompletionCalls
        recoveryPassed = $_.result.recoveryPassed
        processContractPassed = $_.result.processContractPassed
        error = $_.result.error
      }
    })
  }
})

$allSucceeded =
  $runs.Count -gt 0 -and
  $validRuns.Count -eq $runs.Count -and
  $passedRuns.Count -eq $runs.Count
$summary = [ordered]@{
  schemaVersion = 1
  suiteId = $suiteId
  startedAt = $startedAt.ToUniversalTime().ToString("o")
  completedAt = (Get-Date).ToUniversalTime().ToString("o")
  status = if ($allSucceeded) { "passed" } else { "failed" }
  provider = [ordered]@{
    profile = $Profile
    expectedModel = $ExpectedModel
    credentials = "redacted:set"
  }
  configuration = [ordered]@{
    repetitions = $Repetitions
    taskManifests = @($TaskManifests)
  }
  aggregate = [ordered]@{
    requestedRuns = $runs.Count
    validRuns = $validRuns.Count
    passedRuns = $passedRuns.Count
    passRate = if ($validRuns.Count -gt 0) {
      [double]$passedRuns.Count / [double]$validRuns.Count
    } else { $null }
  }
  tasks = $taskSummaries
  infrastructureFailures = @($runs | Where-Object { $null -eq $_.result } | ForEach-Object {
    [ordered]@{
      taskId = $_.taskId
      repetition = $_.repetition
      exitCode = $_.exitCode
      output = $_.runnerOutput
    }
  })
}

$summaryJson = $summary | ConvertTo-Json -Depth 60
$suiteSummaryPath = Join-Path $suiteRoot "summary.json"
[IO.File]::WriteAllText(
  $suiteSummaryPath,
  "$summaryJson`n",
  [Text.UTF8Encoding]::new($false)
)
if ($SummaryPath) {
  $summaryParent = Split-Path -Parent $SummaryPath
  if ($summaryParent) {
    New-Item -ItemType Directory -Path $summaryParent -Force | Out-Null
  }
  [IO.File]::WriteAllText(
    $SummaryPath,
    "$summaryJson`n",
    [Text.UTF8Encoding]::new($false)
  )
}

$summaryJson
if (-not $allSucceeded) {
  exit 1
}
