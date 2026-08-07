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
    $runLabel = "{0:D3}-{1}-{2}" -f $runIndex, $safeTaskId, $repetition
    $childSummary = Join-Path $suiteRoot "$runLabel.json"
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

    $childStdout = Join-Path $suiteRoot "$runLabel.stdout.log"
    $childStderr = Join-Path $suiteRoot "$runLabel.stderr.log"
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
$inconclusiveRuns = @($validRuns | Where-Object {
  $_.result.failureCategory -match '^provider_.*_unavailable$'
})
$scoredRuns = @($validRuns | Where-Object {
  $_.result.failureCategory -notmatch '^provider_.*_unavailable$'
})
$passedRuns = @($scoredRuns | Where-Object { $_.result.status -eq "passed" })

function Get-TimingStats {
  param(
    [Parameter(Mandatory = $true)][object[]]$Runs,
    [Parameter(Mandatory = $true)][string]$Field
  )

  $values = @(
    $Runs | ForEach-Object {
      if ($null -ne $_.result -and $null -ne $_.result.timing) {
        $value = $_.result.timing.$Field
        if ($null -ne $value) {
          [double]$value
        }
      }
    } | Sort-Object
  )

  if ($values.Count -eq 0) {
    return [ordered]@{
      measuredRuns = 0
      sumMs = $null
      meanMs = $null
      medianMs = $null
      minMs = $null
      maxMs = $null
    }
  }

  $sum = [double](($values | Measure-Object -Sum).Sum)
  $middle = [int][Math]::Floor($values.Count / 2)
  $median = if (($values.Count % 2) -eq 1) {
    $values[$middle]
  } else {
    ($values[$middle - 1] + $values[$middle]) / 2
  }

  [ordered]@{
    measuredRuns = $values.Count
    sumMs = [int64]$sum
    meanMs = [double]($sum / $values.Count)
    medianMs = [double]$median
    minMs = [int64]$values[0]
    maxMs = [int64]$values[$values.Count - 1]
  }
}

function Get-TimingSummary {
  param([Parameter(Mandatory = $true)][object[]]$Runs)

  [ordered]@{
    total = Get-TimingStats -Runs $Runs -Field "totalMs"
    phase1 = Get-TimingStats -Runs $Runs -Field "phase1Ms"
    restart = Get-TimingStats -Runs $Runs -Field "restartMs"
    phase2 = Get-TimingStats -Runs $Runs -Field "phase2Ms"
  }
}

$taskSummaries = @($validRuns | Group-Object taskId | ForEach-Object {
  $taskRuns = @($_.Group)
  $taskInconclusive = @($taskRuns | Where-Object {
    $_.result.failureCategory -match '^provider_.*_unavailable$'
  })
  $taskScored = @($taskRuns | Where-Object {
    $_.result.failureCategory -notmatch '^provider_.*_unavailable$'
  })
  $taskPassed = @($taskScored | Where-Object { $_.result.status -eq "passed" })
  [ordered]@{
    taskId = $_.Name
    executedRuns = $taskRuns.Count
    scoredRuns = $taskScored.Count
    inconclusiveRuns = $taskInconclusive.Count
    passedRuns = $taskPassed.Count
    passRate = if ($taskScored.Count -gt 0) {
      [double]$taskPassed.Count / [double]$taskScored.Count
    } else { $null }
    timing = Get-TimingSummary -Runs $taskRuns
    runs = @($taskRuns | ForEach-Object {
      [ordered]@{
        repetition = $_.repetition
        runId = $_.result.runId
        status = $_.result.status
        totalMs = $_.result.timing.totalMs
        phase1Ms = $_.result.timing.phase1Ms
        restartMs = $_.result.timing.restartMs
        phase2Ms = $_.result.timing.phase2Ms
        totalTokens = $_.result.trajectoryMetrics.totalTokens
        completionToolCalls = $_.result.trajectoryMetrics.completionToolCalls
        verifiedPlanCompletionCalls = $_.result.trajectoryMetrics.verifiedPlanCompletionCalls
        recoveryPassed = $_.result.recoveryPassed
        processContractPassed = $_.result.processContractPassed
        failureCategory = $_.result.failureCategory
        error = $_.result.error
      }
    })
  }
})

$allSucceeded =
  $runs.Count -gt 0 -and
  $validRuns.Count -eq $runs.Count -and
  $inconclusiveRuns.Count -eq 0 -and
  $passedRuns.Count -eq $runs.Count
$hasScoredFailure = @($scoredRuns | Where-Object {
  $_.result.status -ne "passed"
}).Count -gt 0
$suiteStatus = if ($allSucceeded) {
  "passed"
} elseif ($inconclusiveRuns.Count -gt 0 -and -not $hasScoredFailure) {
  # An unavailable upstream cannot be scored as a model or task failure.
  "inconclusive"
} else {
  "failed"
}
$completedAt = Get-Date
$timingSummary = Get-TimingSummary -Runs $runs
$timingSummary = [ordered]@{
  wallClockMs = [int64](($completedAt - $startedAt).TotalMilliseconds)
  measuredRuns = $timingSummary.total.measuredRuns
  unmeasuredRuns = $runs.Count - $timingSummary.total.measuredRuns
  total = $timingSummary.total
  phase1 = $timingSummary.phase1
  restart = $timingSummary.restart
  phase2 = $timingSummary.phase2
}
$summary = [ordered]@{
  schemaVersion = 1
  suiteId = $suiteId
  startedAt = $startedAt.ToUniversalTime().ToString("o")
  completedAt = $completedAt.ToUniversalTime().ToString("o")
  status = $suiteStatus
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
    scoredRuns = $scoredRuns.Count
    inconclusiveRuns = $inconclusiveRuns.Count
    passedRuns = $passedRuns.Count
    passRate = if ($scoredRuns.Count -gt 0) {
      [double]$passedRuns.Count / [double]$scoredRuns.Count
    } else { $null }
  }
  timing = $timingSummary
  tasks = $taskSummaries
  infrastructureFailures = @(
    $runs | Where-Object { $null -eq $_.result } | ForEach-Object {
    [ordered]@{
      taskId = $_.taskId
      repetition = $_.repetition
      exitCode = $_.exitCode
      output = $_.runnerOutput
    }
  }
    $inconclusiveRuns | ForEach-Object {
      [ordered]@{
        taskId = $_.taskId
        repetition = $_.repetition
        exitCode = $_.exitCode
        failureCategory = $_.result.failureCategory
        output = $_.result.error
      }
    }
  )
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
